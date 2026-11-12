import AppKit
import ApplicationServices
import Combine
import Foundation
import os
import UserNotifications

/// The last summary failure per meeting, until a summary succeeds or it is cleared.
struct SummaryFailures: Equatable {
    private var byMeeting: [Int64: CoreLLMError] = [:]

    subscript(meetingId: Int64) -> CoreLLMError? { byMeeting[meetingId] }

    /// A success clears, a failure replaces, a call that never ran (no store, invalid usage) leaves it.
    mutating func record<T>(_ outcome: Result<T, CoreLLMError>?, meetingId: Int64) {
        switch outcome {
        case .success?: byMeeting[meetingId] = nil
        case .failure(let error)?: byMeeting[meetingId] = error
        case nil: break
        }
    }

    mutating func clear(meetingId: Int64) { byMeeting[meetingId] = nil }
}

/// FR-CAP-5: global capture on/off, always reflected in the menu bar; FR-ONB-2: the app
/// runs with partial permission grants and reports which capabilities are inactive and why.
@MainActor
final class AppState: ObservableObject {
    @Published private(set) var activityCaptureEnabled = false
    /// Set while a meeting is actively being recorded — the row id to complete when the
    /// meeting ends, and its title for display. All 3 trigger paths (manual
    /// `startManualMeeting()`, calendar/detected via `startCapture(for:)`) produce a real
    /// row here, independent of activity capture's own on/off state (FR-MTG-14).
    @Published private(set) var activeMeetingId: Int64?
    /// Start of the active meeting (the original start for a continued one); drives the menu bar elapsed time.
    @Published private(set) var activeMeetingStartedAt: Date? {
        didSet { restartMenuBarElapsedTimer() }
    }
    /// The menu bar's recording label ("1:05"), nil when nothing records. Refreshed by a
    /// plain 30 s timer, not a TimelineView: inside the MenuBarExtra label a TimelineView
    /// rescheduled itself on every redraw (macOS 26), a main-thread loop that froze the
    /// app and grew memory without bound for as long as a meeting recorded.
    @Published private(set) var menuBarElapsed: String?
    private var menuBarElapsedTimer: Timer?
    @Published private(set) var activeMeetingTitle: String?
    /// The active meeting continues an earlier one (a restart within the continue window;
    /// see `MeetingContinuation`). The recording header shows "Continued".
    @Published private(set) var activeMeetingIsContinued = false
    /// The active meeting's current part (1-based), completed in `endMeeting`.
    private var activePartIndex: Int64?
    /// The detected app the active meeting came from, told in `endMeeting` so a rejoin
    /// is detected again.
    private var activeMeetingBundleID: String?
    /// The calendar event the active meeting records, told in `endMeeting` so a rejoin
    /// before the event's end prompts again as the same calendar meeting.
    private var activeCalendarEvent: MeetingContinuation.CalendarEvent?
    /// The calendar meeting that ended last: a detected rejoin of its call continues it
    /// (`MeetingContinuation.calendarFallbackMatches`).
    private var lastEndedCalendarMeeting: MeetingContinuation.EndedCalendarMeeting?
    /// The notes typed into the meeting that ended last, carried into its next part if it
    /// continues before part 1's transcript has stored them.
    private var lastEndedMeetingNotes: (id: Int64, notes: String)?
    /// The active recording's call link (its event's, or a browser detection's): what
    /// `MeetingSplit` compares the next calendar meeting's link against.
    private var activeCallLink: DetectedConferenceLink?
    /// Polls a detected meeting's own app UI for a running participant-count hint
    /// (see `ParticipantCountEstimator`) — started in `finishStartingCapture` for
    /// `.detected`-sourced meetings only, stopped/cleared in `endMeeting`. Calendar
    /// and manual meetings never set this — they already have a real attendee count
    /// or none at all.
    private var participantCountEstimator: ParticipantCountEstimator?
    /// Saves shared slides from the meeting window while any meeting records. Created for
    /// every meeting by `startMeetingSnapshots`, running only while `snapshotSettings.enabled`
    /// (the setting pauses and resumes it); released in `endMeeting`.
    private var meetingSnapshotService: MeetingSnapshotService?
    /// FR-MTG-5: in-meeting notes, bindable from the notes pad shown while a meeting is
    /// actively recording. In-memory only for this pass — not persisted across an app
    /// crash mid-meeting. Captured and cleared in `stopCapture()`.
    @Published var meetingNotes: String = ""
    /// Timeline's Recent list hides screen-text capture items by default (meetings only)
    /// since raw screen scrapes read as noisy/sensitive next to meeting records — this
    /// toggle opts back in. Local-only display preference, not synced across devices,
    /// matching this app's existing simple-local-setting pattern (see `SyncState.deviceName`).
    @Published var showScreenCaptures: Bool = UserDefaults.standard.bool(forKey: "com.mpointer.kestrel.showScreenCaptures") {
        didSet {
            UserDefaults.standard.set(showScreenCaptures, forKey: "com.mpointer.kestrel.showScreenCaptures")
        }
    }
    /// MeetingDetailView's transcript/list body text size — the summary editor uses
    /// `readingFontSize + 2`, preserving that view's original 13pt/15pt size
    /// relationship. Local-only display preference, not synced across devices, matching
    /// `showScreenCaptures`'s own simple-local-setting pattern. Default (13) matches the
    /// smaller size's original hardcoded value.
    @Published var readingFontSize: Double = UserDefaults.standard.object(forKey: "com.mpointer.kestrel.readingFontSize") as? Double ?? 13 {
        didSet {
            UserDefaults.standard.set(readingFontSize, forKey: "com.mpointer.kestrel.readingFontSize")
        }
    }
    /// FR-MTG-5: which meeting `transcriptionQueue` is currently working on, and
    /// which stage — nil/nil when nothing is in flight. The automatic pipeline runs
    /// entirely on background queues with no other visible progress signal, which was a
    /// real source of user confusion (no way to tell it was even running).
    @Published private(set) var processingMeetingId: Int64?
    @Published private(set) var processingStage: String?
    /// Why each meeting's last summary attempt (automatic or Regenerate) failed, until one
    /// succeeds. In memory only: core keeps no per-meeting failure, so a relaunch forgets it.
    @Published private(set) var summaryFailures = SummaryFailures()

    private func recordSummaryOutcome<T>(_ outcome: Result<T, CoreLLMError>?, meetingId: Int64) {
        summaryFailures.record(outcome, meetingId: meetingId)
    }

    /// Drops a meeting's kept failure: the user changed what the next summary would use, or
    /// the meeting is gone.
    func clearSummaryFailure(meetingId: Int64) {
        summaryFailures.clear(meetingId: meetingId)
    }

    /// Bumped by every `startCapture`/`beginManualCapture`/`stopCapture` call.
    /// `finishStartingCapture`/`finishStartingManualCapture` compare their own captured
    /// value against the current one before proceeding — if they differ, a stop (or a
    /// newer start) happened during the async `insertMeeting` round-trip, and that
    /// continuation must NOT resume capture or overwrite state a later call already set
    /// up.
    private var captureGeneration = 0
    /// Synchronous re-entrancy guard for meeting-starting specifically (manual,
    /// calendar, and detected all share this) -- set true immediately when a start
    /// begins, before any async `insertMeeting` round-trip, so a rapid second start
    /// attempt during that gap is blocked even though `activeMeetingId` itself isn't
    /// set until the round-trip completes. Cleared at the top of
    /// `finishStartingCapture`/`finishStartingManualCapture`, regardless of outcome.
    /// Previously this role was served by `captureEnabled` itself (shared with
    /// activity capture) -- now that the two are independent, meeting-starting needs
    /// its own dedicated lock.
    private var isStartingMeeting = false
    /// Set once by `TimelineView.swift`'s `.onAppear` (which has real access to
    /// `@Environment(\.openWindow)`) — used by the meeting-alert panel's "Start
    /// Recording & Open Timeline" action. `AppState` itself has no SwiftUI environment
    /// access of its own.
    var openTimeline: (() -> Void)?
    /// Opens the Settings window; set by `WhimbrelShellView`'s `.onAppear` alongside `openTimeline`.
    var openSettingsWindow: (() -> Void)?
    /// Opens a meeting's window; set by `TimelineView`'s `.onAppear` alongside `openTimeline`.
    private var openMeetingWindow: ((Int64) -> Void)?
    /// A meeting asked for (alert click, `whimbrel://` link) before the opener was set.
    private var meetingToOpen: Int64?
    /// Where an opened meeting should seek to, in centiseconds; the meeting page consumes it.
    @Published var pendingMeetingSeek: (meetingId: Int64, atCs: Int64)?
    /// The shell's selected tab, here rather than in `WhimbrelShellView` so other screens
    /// (Today's "Review" button) can switch tabs.
    @Published var selectedTab: WhimbrelTab = .today
    /// The topic the Topics tab shows; nil picks the first one.
    @Published var selectedTopicId: Int64?

    /// Shows topic `id` on the Topics tab (a topic chip, a link).
    func openTopic(_ id: Int64) {
        selectedTopicId = id
        selectedTab = .topics
        openTimeline?()
    }
    /// Set by the ⌘F and ⌘N menu commands and cleared by Library once it acts on them.
    /// Flags rather than events, so a request made before Library is on screen still
    /// lands when it appears.
    @Published var pendingLibrarySearch = false
    /// A Settings section to show the next time Settings is shown (or now, if it is open).
    @Published var pendingSettingsSection: SettingsSection?
    /// Set to show Settings → Storage's Free up space sheet (the low-disk notice and line).
    @Published var showFreeUpSpace = false
    private var lowDiskTimer: Timer?
    @Published var pendingNewConversation = false
    /// Text from Today's composer, sent as the first message of the new conversation.
    @Published var pendingAskPrompt: String?
    /// The Meeting Prep brief the main window shows in a sheet (a notification click, Routines' "Open brief").
    @Published var prepBriefToShow: PrepBriefRequest?

    /// Opens (or, if open, closes) the hotkey panel for `frontmost`.
    func toggleHotkeyPanel(frontmost: HotkeyPanelModel.Frontmost?) {
        hotkeyPanel.toggle(frontmost: frontmost)
    }

    /// The menu-bar entry: the app the user was in before the menu opened (Whimbrel itself
    /// if that is where they were, which the panel reports as excluded).
    func openHotkeyPanelFromMenu(menuOpenedAt: Date) {
        let frontmost = frontmostTracker.appBeforeMenu(openedAt: menuOpenedAt, ownBundleId: Bundle.main.bundleIdentifier)
        if hotkeyPanel.isVisible { hotkeyPanel.close() }
        toggleHotkeyPanel(frontmost: frontmost)
    }

    /// The panel's route, decided without the question (the destination line is shown
    /// before one is typed): the default provider, never the Ask tab's per-question
    /// Ollama substitution; Ollama (or the bundled model) in local-only mode.
    func windowAskRouting() -> WindowAskRouting {
        let selection = routing.defaultSelection()
        let localOnly = routing.localOnlyMode
        let provider: RoutingSettings.Provider = localOnly ? .ollama : selection.provider
        let model = provider == .ollama ? selection.ollamaModel : selection.hostedModel
        let config = routing.routingConfig(
            preferredProvider: provider, preferredModel: model,
            customInstructions: promptLibrary.instructions[.ask] ?? "")
        let stream = askStreamRoute(
            provider: provider, modelLabel: model, localOnlyMode: localOnly,
            replacesOllama: LocalModel.replacesOllama(routing),
            whimbrelCredentials: syncState.whimbrelCredentials,
            loadKey: { KeychainService.load(for: $0) })
        return WindowAskRouting(config: config, stream: stream,
                                localModelAvailable: !routing.ollamaModels.isEmpty || LocalModel.isInstalled)
    }

    func showPrepBrief(key: String) {
        NSApp.activate(ignoringOtherApps: true)
        openTimeline?()
        prepBriefToShow = PrepBriefRequest(key: key)
    }

    /// Opens meeting `id` (an alert notification or a `whimbrel://meeting/<id>?at=` link) and
    /// leaves `atCs` (meeting-absolute) in `pendingMeetingSeek` for the meeting page.
    func openMeeting(id: Int64, at atCs: Int64) {
        pendingMeetingSeek = (id, atCs)
        NSApp.activate(ignoringOtherApps: true)
        guard let openMeetingWindow else {
            meetingToOpen = id
            return
        }
        openMeetingWindow(id)
    }

    /// Called once the meeting window can be opened; opens one asked for before then.
    func setMeetingWindowOpener(_ open: @escaping (Int64) -> Void) {
        openMeetingWindow = open
        if let id = meetingToOpen {
            meetingToOpen = nil
            open(id)
        }
    }

    func handleOpenURL(_ url: URL) {
        guard let link = DeepLink.parse(url) else { return }
        open(link)
    }

    /// Opens a deep link's meeting. A link another device made (the alert email) resolves
    /// through `sync_meeting_map`; one that doesn't resolve opens the meeting window's
    /// "no longer available" page, or, with `activateIfUnresolved` (a notification tap), just
    /// brings the app forward.
    func open(_ link: DeepLink.Target, activateIfUnresolved: Bool = false) {
        guard let origin = link.origin, origin != deviceId else { return openMeeting(id: link.meetingId, at: link.atCs) }
        let store = coreStore
        let deviceId = deviceId
        CoreMemoryStore.accessQueue.async {
            let local = DeepLink.localMeetingId(link, deviceId: deviceId) { store?.resolveSyncMeeting(peerDeviceId: $0, sourceMeetingId: $1) }
            // ponytail: -1 is never a meeting id, so the window shows "no longer available".
            DispatchQueue.main.async { [weak self] in
                if local == nil, activateIfUnresolved { NSApp.activate(ignoringOtherApps: true) } else { self?.openMeeting(id: local ?? -1, at: link.atCs) }
            }
        }
    }

    /// Sends `budgets` to core on the access queue, then posts `providerBudgetsPushed` once they are stored.
    private func pushProviderBudgets(_ budgets: [String: Int]) {
        // Used only on `accessQueue`, which serializes every store call.
        guard let coreStore else { return }
        nonisolated(unsafe) let store = coreStore
        CoreMemoryStore.accessQueue.async {
            guard store.setProviderBudgets(budgets) else { return }
            DispatchQueue.main.async { NotificationCenter.default.post(name: AppState.providerBudgetsPushed, object: nil) }
        }
    }

    /// Posted on the main thread after the monthly budgets reach core (launch, a change, a sync pull).
    static let providerBudgetsPushed = Notification.Name("com.mpointer.kestrel.providerBudgetsPushed")

    /// Posts budget notices after each governed AI call, each settings or content sync pull,
    /// and each budget push to core.
    private func startBudgetNotices() {
        // Used only on `accessQueue`, which serializes every store call.
        guard let coreStore else { return }
        nonisolated(unsafe) let store = coreStore
        BudgetNotices.registerCategory()
        let check = { CoreMemoryStore.accessQueue.async { BudgetNotices.check(store) } }
        CoreMemoryStore.afterGovernedCall = check
        NotificationCenter.default.publisher(for: AppState.providerBudgetsPushed).map { _ in () }
            .merge(with: settingsSyncCoordinator.$lastSyncedAt.dropFirst().map { _ in () },
                   contentSyncCoordinator.$lastSyncedAt.dropFirst().map { _ in () })
            .sink { check() }
            .store(in: &cancellables)
    }

    func requestLibrarySearch() {
        openTimeline?()
        selectedTab = .library
        pendingLibrarySearch = true
    }

    func requestNewConversation(prompt: String? = nil) {
        openTimeline?()
        selectedTab = .library
        pendingAskPrompt = prompt
        pendingNewConversation = true
    }

    let permissions = PermissionsManager()
    let audio = AudioCaptureService()
    let screenText = ScreenTextCaptureService()
    let calendar = CalendarService()
    let googleCalendarPolling = GoogleCalendarPollingService()
    let gmailPolling = GmailPollingService()
    /// EventKit's meetings merged with the Calendar connector's, deduped by exact
    /// start-time + case-insensitive title match (core/api::connectors::calendar's
    /// `merge_meetings`, via `CoreMemoryStore.mergeMeetings`) -- the real source every meeting-detection consumer (the
    /// Schedule panel, the capture-prompt trigger) reads, instead of either source
    /// directly.
    var allUpcomingMeetings: [QualifyingMeeting] {
        Self.todaysMeetings(eventKit: calendar.upcomingMeetings, oauth: googleCalendarPolling.oauthMeetings, now: Date())
    }

    /// What the Today panel and the At a Glance schedule show: `allUpcomingMeetings` without
    /// the meetings the user declined and the all-day events, the same rule Upcoming uses.
    /// The meeting trigger and the other consumers keep reading `allUpcomingMeetings`.
    var todayPanelMeetings: [QualifyingMeeting] {
        Self.todaysMeetings(eventKit: calendar.upcomingMeetings, oauth: googleCalendarPolling.oauthMeetings, now: Date(), forPanel: true)
    }

    /// The connector keeps every future occurrence of a recurring meeting (for years
    /// ahead), while EventKit covers only the rest of today, so the connector's list is
    /// cut to the same window before merging. `forPanel` also drops all-day events and
    /// declined meetings; it is off for the trigger, which must still see them.
    nonisolated static func todaysMeetings(eventKit: [QualifyingMeeting], oauth: [QualifyingMeeting], now: Date, forPanel: Bool = false, calendar: Calendar = .current) -> [QualifyingMeeting] {
        let end = CalendarService.todayWindowEnd(now: now, calendar: calendar)
        // All-day events are dropped before the merge: `isAllDay` is not part of the JSON that
        // round-trips through core, so a merged meeting always reads false. EventKit is the only
        // source that needs it; Google all-day events never get here (core skips events with no
        // dateTime). Declined is on the wire, so it is filtered after the merge.
        let merged = CoreMemoryStore.mergeMeetings(eventKit: forPanel ? eventKit.filter { !$0.isAllDay } : eventKit, oauth: oauth.filter { $0.startDate < end })
        return forPanel ? merged.filter { $0.selfResponse != .declined } : merged
    }

    /// How far ahead the phones get calendar events: today plus tomorrow, so a reminder
    /// for an early meeting can fire before the Mac is awake.
    static let calendarSyncHorizon: TimeInterval = 48 * 3600

    /// What calendar sync sends to the phones. Separate from `todaysMeetings` on purpose: that
    /// one stays today-only for the Mac's own surfaces. Today's already-started meetings stay
    /// (EventKit's lookbehind); only the upper bound is cut, at `now + calendarSyncHorizon`.
    /// The week list can reach 7 days out and the connector's years, so every source is cut.
    /// Meetings the user declined are left out (phone reminders skip them), the same rule as
    /// `weekMeetings`; no response, accepted and tentative stay.
    nonisolated static func syncMeetings(eventKit: [QualifyingMeeting], week: [QualifyingMeeting], oauth: [QualifyingMeeting], now: Date) -> [QualifyingMeeting] {
        let end = now.addingTimeInterval(calendarSyncHorizon)
        return CoreMemoryStore.mergeMeetings(eventKit: eventKitSyncSide(eventKit: eventKit, week: week, now: now), oauth: oauth.filter { $0.startDate < end })
            .filter { $0.selfResponse != .declined }
    }

    /// The EventKit side of `syncMeetings`, before it is merged with Google: cut at the sync
    /// horizon and deduped. An EventKit id is shared by every occurrence of a recurring series,
    /// so dedupe on id and start together, the same identity the items table uses
    /// ("{id}#{start_secs}").
    nonisolated private static func eventKitSyncSide(eventKit: [QualifyingMeeting], week: [QualifyingMeeting], now: Date) -> [QualifyingMeeting] {
        let end = now.addingTimeInterval(calendarSyncHorizon)
        var seen = Set<String>()
        return (eventKit + week).filter { $0.startDate < end && seen.insert("\($0.id)#\(Int($0.startDate.timeIntervalSince1970))").inserted }
    }

    /// Guards against a quiet empty read (a store that has not loaded, a wake-up) hiding
    /// everything: every empty list, including the first one in a run, is withheld until a
    /// second distinct refresh agrees. Non-empty lists pass straight through; nil (no
    /// successful fetch) passes through and resets the streak. `previousCount` no longer
    /// matters (a launch with nothing known yet needs the same confirmation).
    nonisolated static func confirmedPresence(
        current: [PresentEvent]?, previousCount: Int?, emptyStreak: Int, generation: Int, lastGeneration: Int
    ) -> (present: [PresentEvent]?, emptyStreak: Int, lastGeneration: Int) {
        guard let current else { return (nil, 0, lastGeneration) }
        guard current.isEmpty else { return (current, 0, lastGeneration) }
        // The streak counts EventKit refreshes (`generation`), not persist calls: one refresh
        // publishes twice and a Google poll persists again.
        let streak = generation == lastGeneration ? emptyStreak : emptyStreak + 1
        return (streak >= 2 ? current : nil, streak, generation)
    }

    /// Every Apple Calendar meeting this fetch shows, for the presence table: the EventKit side
    /// before merging with Google (a meeting with a Google twin is still present in Apple
    /// Calendar), declined ones left out. Nil unless the fetch succeeded, so a revoked
    /// permission or a failed fetch reconciles nothing; an empty list is a valid answer.
    nonisolated static func eventKitPresence(eventKit: [QualifyingMeeting], week: [QualifyingMeeting], now: Date, fetchSucceeded: Bool) -> [PresentEvent]? {
        guard fetchSucceeded else { return nil }
        return eventKitSyncSide(eventKit: eventKit, week: week, now: now)
            .filter { $0.selfResponse != .declined }
            .compactMap { CoreMemoryStore.calendarEventItem(for: $0, deviceId: "presence") }
            .map { PresentEvent(externalId: $0.externalId, startUnixMs: $0.tsUnixMs) }
    }

    /// Today's "Upcoming" section: the 7 days after today from both sources, merged the same way,
    /// declined meetings left out. Recomputed when either source publishes (see `init`), not per
    /// render. Nothing else reads it (not the trigger, not calendar sync).
    @Published private(set) var upcomingWeekMeetings: [QualifyingMeeting] = []

    /// Both sources are cut to the window here: EventKit's list is up to a refresh (60 s) old, so
    /// after midnight it can still hold the day that just became today.
    nonisolated static func weekMeetings(eventKit: [QualifyingMeeting], oauth: [QualifyingMeeting], now: Date) -> [QualifyingMeeting] {
        let week = CalendarService.weekWindow(now: now)
        return CoreMemoryStore.mergeMeetings(eventKit: eventKit.filter { week.contains($0.startDate) }, oauth: oauth.filter { week.contains($0.startDate) })
            .filter { $0.selfResponse != .declined }
    }
    let meetingTrigger = MeetingTriggerService()
    /// "Away Mode": join Zoom meetings unattended. Owns the keep-awake assertion while on.
    /// Downloads missing models at launch, with no prompt (doc 17, item 1.1).
    let modelBootstrap = ModelBootstrap()
    let localModelDownload = ModelBootstrap(
        present: { LocalModel.isInstalled },
        download: { try await ModelDownloadService.downloadLocalModel(progress: $0) }
    )
    let turboDownload = ModelBootstrap(
        present: { FileManager.default.fileExists(atPath: TranscriptionService.turboModelPath.path) },
        download: { try await ModelDownloadService.downloadTurbo(progress: $0) }
    )
    let awayMode = AwayModeSettings(keepAwake: KeepAwake(), available: AppState.awayZoomJoinEnabled)
    /// Away Mode's join logic. Eligible calendar prompts go here instead of the alert panel.
    private(set) lazy var awayCoordinator = AwayModeCoordinator(
        env: LiveAwayEnvironment(app: self), isEnabled: { [awayMode] in awayMode.enabled }
    )
    let meetingHeuristicDetector = MeetingHeuristicDetector()
    let meetingEndDetector = MeetingEndDetector()
    let meetingAlertPanel = MeetingAlertPanelController()
    /// Global hotkey for the ask-window panel (Carbon, no permission). Task 5 sets `onHotkey`.
    let hotkeyService = HotkeyService(registrar: CarbonHotkeyRegistrar(), defaults: .standard)
    var onHotkey: (() -> Void)?
    /// The "Ask about this window" panel, opened by the hotkey or the menu-bar item.
    private(set) lazy var hotkeyPanel: HotkeyPanelController = {
        let model = HotkeyPanelModel(
            reader: FocusedWindowReader(provider: AXFocusedElementProvider()),
            store: { [weak self] in self?.coreStore },
            routing: { [unowned self] in self.windowAskRouting() },
            beforeAsk: { [weak self] route in
                // As the Ask tab: Whimbrel AI needs a live sync ID token. On Return only.
                guard let self, route.config.preferredProvider == RoutingSettings.Provider.whimbrel.rawValue,
                      !route.config.sensitive, self.syncState.whimbrelCredentialsNeedRefresh else { return }
                await self.syncState.refreshWhimbrelCredentials()
            })
        let controller = HotkeyPanelController(model: model)
        controller.openAIProviders = { [weak self] in
            // Set first: SettingsView takes it on appear, or on change when already open.
            self?.pendingSettingsSection = .ai
            NSApp.activate(ignoringOtherApps: true)
            self?.openSettingsWindow?()
        }
        return controller
    }()
    /// The app frontmost before Whimbrel, for the menu-bar entry to the panel.
    let frontmostTracker = FrontmostAppTracker()
    let retention = RetentionSettings()
    let retentionScheduler = RetentionScheduler()
    let routineScheduler = RoutineScheduler()
    /// Task 7: spawns/stops the real `whimbrel-mcp-server` binary, driven by the "Enable
    /// MCP Server" Settings toggle (`mcpServerProcess.enabled`, wired to `start()`/`stop()`
    /// below in `init`, same pattern as `meetingTrigger.$heuristicDetectionEnabled`).
    let mcpServerProcess = McpServerProcessManager()
    var routing = RoutingSettings()
    let replyPriority = ReplyPrioritySettings()
    let topicSort = TopicSortSettings()
    /// "Find steps with TypeSafe" (Settings > Topics): same Start flow as topic sorting.
    lazy var topicSteps = TopicSortSettings(enabledKey: "com.mpointer.kestrel.topicStepsEnabled", startedKey: "com.mpointer.kestrel.topicStepsStarted")
    /// The last local steps run found no local model (or an unreachable one).
    @Published var topicStepsNoModel = false
    let meetingPrepSettings = MeetingPrepSettings()
    let meetingTitleCheck = MeetingTitleCheckSettings()
    /// "Live transcript during meetings" (on by default) and the service that transcribes a
    /// recording in chunks while it is being written
    /// (docs/superpowers/specs/2026-10-06-live-transcription-design.md).
    let liveTranscriptSettings = LiveTranscriptSettings()
    /// "Alert me when someone asks me something" (docs/superpowers/specs/2026-11-02-live-question-alerts-design.md).
    /// Needs live transcription, so it starts it even with the live transcript setting off.
    let liveQuestionSettings = LiveQuestionSettings()
    /// "Remember voices" for naming speakers (docs/superpowers/specs/2026-11-05-speaker-names-design.md).
    let speakerNamesSettings = SpeakerNamesSettings()
    /// The active recording's question watcher; nil when alerts are off or no name is set.
    private var liveQuestionWatcher: LiveQuestionWatcher?
    /// Alerts go to a notification, an email to the user's own Gmail and, through sync, the
    /// other devices. Lazy: it needs the stores and connector state set up in `init`.
    lazy var liveAlertDelivery: LiveAlertDelivery = makeLiveAlertDelivery()
    /// The notification center's delegate (it holds it weakly).
    private var notificationClickHandler: NotificationClickHandler?
    /// Alerts start live transcription only when they can run (a name is set), so a user who
    /// turned the live transcript off isn't transcribed for nothing.
    private var liveTranscriptionWanted: Bool {
        Self.liveTranscriptionWanted(transcript: liveTranscriptSettings.enabled, alerts: liveQuestionSettings.enabled, names: profile.liveQuestionNames)
    }

    nonisolated static func liveTranscriptionWanted(transcript: Bool, alerts: Bool, names: [String]) -> Bool {
        transcript || (alerts && !names.isEmpty)
    }
    let recordingNoticeSettings = RecordingNoticeSettings()
    /// Nil until Task 7b's real Zoom chat poster: the notice is a reminder until then.
    let recordingNoticeChat: ZoomChatPosting? = nil
    /// "Ask if I'm still recording after N hours" and its panel
    /// (docs/superpowers/specs/2026-10-24-recording-failsafes-design.md §2).
    let recordingCheckInSettings = RecordingCheckInSettings()
    let recordingCheckInPanel = RecordingCheckInPanelController()
    /// The active meeting's check-in, polled every 5 s by `checkInTimer`; nil when no meeting.
    private var activeCheckIn: RecordingCheckIn?
    private var checkInTimer: Timer?
    /// "Still in a meeting?" (`StillInMeeting`) for a recording started by hand, fed by the
    /// same 5 s poll and on wake; nil for any other recording.
    private var stillInMeeting: (promptShownAt: Date?, keptAt: Date?, polledAt: Date?)?
    private var wakeObserver: NSObjectProtocol?
    /// "Whimbrel isn't hearing anything": silence on every track while recording, fed by the
    /// same 5 s poll. `silenceWarning` is the inline banner's text; nil while sound is arriving.
    private var silenceWatch: SilenceWatch?
    @Published private(set) var silenceWarning: String?
    /// "Meeting snapshots": slide images from the meeting window (on by default).
    let snapshotSettings = SnapshotSettings()
    /// Storage retention: the media folder (audio and snapshots) and the media ages.
    let mediaSettings = MediaSettings()
    /// Storage retention's media run: daily and at launch (`retentionScheduler`), when a drive
    /// mounts, and after each meeting finishes processing. No store in a test host, so it
    /// never touches real files there.
    lazy var mediaRetention = MediaRetentionRunner(
        store: KeychainAccess.isTestHost ? nil : coreStore, settings: mediaSettings, deviceId: deviceId,
        isBusy: { [weak self] id in self.map { $0.isStartingMeeting || $0.mediaBusyMeetingIds.contains(id) } ?? true },
        busyIds: { [weak self] in self?.mediaBusyMeetingIds ?? [] }
    )
    /// Meetings media retention must not touch now: recording, transcribing (or queued),
    /// summarizing, or waiting for a split's summary.
    var mediaBusyMeetingIds: Set<Int64> {
        var ids = Set(transcriptionQueue?.pendingMeetingIds ?? []).union(regeneratingSummaryIds)
        for id in [activeMeetingId, processingMeetingId, pendingSplitSummary] { if let id { ids.insert(id) } }
        return ids
    }
    /// Meetings whose summary `regenerateMeetingSummary` is rewriting.
    private var regeneratingSummaryIds: Set<Int64> = []
    private var mountObserver: NSObjectProtocol?
    /// System / Light / Dark, per device.
    let appearanceSettings = AppearanceSettings()
    /// The Settings skin chat. Owned here, not by its view, so the chat survives the Settings
    /// window rebuild (`ThemedRoot`) that every skin it switches on causes.
    lazy var skinChat = SkinChatSession(
        skinStore: .shared, userSkins: .shared, backend: CoreSkinTurnBackend(routing: routing, deviceId: deviceId), routing: routing
    )
    /// Multilingual mode transcribes live with the multilingual model, detecting the language.
    private static var liveDetectsLanguage: Bool {
        MeetingTranscriptionQueue.usesMultilingual(
            enabled: AppLanguage.multilingual(),
            modelPresent: FileManager.default.fileExists(atPath: TranscriptionService.multilingualModelPath.path))
    }
    lazy var liveTranscript = LiveTranscriptionService(
        store: { [weak self] in self?.coreStore },
        engine: { TranscriptionEngines.live(detecting: AppState.liveDetectsLanguage) },
        detectLanguage: { AppState.liveDetectsLanguage },
        systemURL: { [weak self] in self?.audio.systemAudioFileURL },
        micURL: { [weak self] in self?.audio.micAudioFileURL }
    )
    /// Shared handle onto the Rust-backed memory store (NFR-PRT-1), used by the
    /// timeline view and the core self-test alike.
    let coreStore = CoreMemoryStore(
        path: SpikePaths.database,
        modelsDir: EmbeddingModelLocator.modelsDirectoryPath().map(URL.init(fileURLWithPath:))
    )
    /// A second connection for calls that hold the store across an LLM request (the slow
    /// lane). Used only on `CoreMemoryStore.slowLaneQueue`, so Ollama never blocks
    /// `accessQueue` and the UI reads behind it.
    let slowStore: CoreMemoryStore?
    let syncState = SyncState()
    /// Constructed in `init()` (not as property initializers) so they can receive
    /// `coreStore`: their connected-account registries live in the core's
    /// `connector_account` table (M3.0 Task 9).
    let calendarConnectorState: CalendarConnectorState
    let gmailConnectorState: GmailConnectorState
    let outlook365ConnectorState: Outlook365ConnectorState
    let settingsSyncCoordinator: SettingsSyncCoordinator
    let contentSyncCoordinator: ContentSyncCoordinator
    let updateChecker = UpdateChecker()
    let promptLibrary = PromptLibrarySettings()
    /// Who the user is (fields + facts); synced, and sent to core as `RoutingConfig.profile`.
    let profile = ProfileSettings()
    /// Which device runs routines (synced); empty means every device does.
    let routineHost = RoutineHostSettings()
    /// Suggested Profile facts waiting for Add or Dismiss (local to this device).
    lazy var profileSuggestions = ProfileSuggestionsModel(
        profile: profile,
        list: { [store = coreStore] in await Self.onAccessQueue { store?.listProfileSuggestions() ?? [] } },
        dismiss: { [store = coreStore] id in await Self.onAccessQueue { store?.dismissProfileSuggestion(id: id) ?? false } },
        delete: { [store = coreStore] id in await Self.onAccessQueue { store?.deleteProfileSuggestion(id: id) ?? false } }
    )

    private nonisolated static func onAccessQueue<T>(_ work: @escaping () -> T) async -> T {
        await withCheckedContinuation { continuation in
            CoreMemoryStore.accessQueue.async { continuation.resume(returning: work()) }
        }
    }

    /// This device's own stable identity — its sync-enrollment X25519 public key,
    /// generated eagerly (not lazily at "Enable Sync" time) so it's stable from this
    /// install's very first captured row, whether or not the user ever enables sync.
    /// Falls back to the literal "spike-device" only if local keypair generation
    /// itself fails (e.g. Keychain access denied) — rows written under that fallback
    /// are indistinguishable from every pre-Task-1 install's rows and are therefore
    /// permanently un-syncable, same as this project's explicit decision for
    /// already-shipped historical rows.
    let deviceId: String

    private var cancellables = Set<AnyCancellable>()
    /// Reads defaults.json a minute after launch, then daily (`startModelDefaults`).
    private var modelDefaultsTask: Task<Void, Never>?
    private var terminationObserver: NSObjectProtocol?
    /// Meetings the quit path completed without transcribing; launch transcribes each once.
    private static let pendingLaunchTranscriptionKey = "com.mpointer.kestrel.pendingLaunchTranscription"

    init() {
        // Themes and skins: install the bundled and user ones (skin palettes are themes, from
        // skins/palettes/ and each user skin's palettes/) and apply the stored appearance before
        // the first window draws.
        SkinStore.shared.installBundledAndUserSkins()
        appearanceSettings.apply()
        routing.profileProvider = { [profile] in profile.routingProfile }
        routing.whimbrelCredentialsProvider = { [syncState] in syncState.whimbrelCredentials }
        routing.whimbrelModelsFetcher = { [syncState] in try await syncState.fetchAIModels() }
        slowStore = coreStore.flatMap { primary in
            CoreMemoryStore.accessQueue.sync { CoreMemoryStore(secondaryTo: primary, path: SpikePaths.database) }
        }
        if coreStore != nil, slowStore == nil {
            Logger(subsystem: "com.mpointer.kestrel", category: "core-bridge").error("Could not open the slow-lane store; summaries, digests, routines and asks are unavailable.")
        }
        calendarConnectorState = CalendarConnectorState(coreStore: coreStore)
        gmailConnectorState = GmailConnectorState(coreStore: coreStore)
        outlook365ConnectorState = Outlook365ConnectorState(coreStore: coreStore)
        deviceId = syncState.ensureLocalDeviceIdentity() ?? "spike-device"
        // Queued first, so it runs before any store work enqueued below (serial queue);
        // async, so a long job already on the queue can't block launch. Locals rather
        // than a capture list: `self` is not fully initialised yet.
        let store = coreStore
        let ownDeviceId = deviceId
        let rerootPrefixes = SpikePaths.rerootPrefixes
        let newPrefix = SpikePaths.root.path + "/"
        CoreMemoryStore.accessQueue.async {
            for old in rerootPrefixes {
                let n = store?.replaceMeetingPathPrefix(old: old, new: newPrefix) ?? -1
                let log = Logger(subsystem: "com.mpointer.kestrel", category: "data-folder")
                if n < 0 { log.error("Re-rooting stored file paths from \(old, privacy: .public) failed; it retries on the next launch.") }
                else if n > 0 { log.notice("Re-rooted \(n) stored file-path rows from \(old, privacy: .public).") }
            }
            // Without its own id the store can't name this device's meetings: Forget then removes a
            // meeting here only, and no other device hears of it.
            if let store, !store.setDeviceId(ownDeviceId) {
                Logger(subsystem: "com.mpointer.kestrel", category: "sync")
                    .error("This Mac's device id isn't set on the store; Forget meeting won't reach other devices.")
            }
        }
        let slow = slowStore
        CoreMemoryStore.slowLaneQueue.async {
            if let slow, !slow.setDeviceId(ownDeviceId) {
                Logger(subsystem: "com.mpointer.kestrel", category: "sync")
                    .error("This Mac's device id isn't set on the slow-lane store.")
            }
        }
        for key in ["com.mpointer.kestrel.transcriptionModelPath",
                    "com.mpointer.kestrel.diarizationSegmentationModelPath",
                    "com.mpointer.kestrel.diarizationEmbeddingModelPath"] {
            if let path = UserDefaults.standard.string(forKey: key), let rerooted = SpikePaths.rerooted(path) {
                UserDefaults.standard.set(rerooted, forKey: key)
            }
        }
        AccessQueueWaitProbe.start()
        settingsSyncCoordinator = SettingsSyncCoordinator(routing: routing, retention: retention, promptLibrary: promptLibrary, profile: profile, routineHost: routineHost, syncState: syncState, coreStore: coreStore)
        contentSyncCoordinator = ContentSyncCoordinator(coreStore: coreStore, syncState: syncState, deviceId: deviceId)
        SkinSync.connect(store: coreStore, skins: .shared)
        syncState.resetPushCursor = { [store = coreStore] in
            await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
                CoreMemoryStore.accessQueue.async {
                    continuation.resume(returning: store?.resetSyncPushCursor() ?? false)
                }
            }
        }
        reconcileOrphanedRecordingMeetings()
        startModelDefaults()
        startLowDiskChecks()
        // `queue: nil` runs the block synchronously on the posting (main) thread, before
        // `terminate()` exits; see McpServerProcessManager.
        terminationObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.willTerminateNotification, object: nil, queue: nil
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.endMeetingForQuit()
                self?.meetingHeuristicDetector.stop()
            }
            CoreMemoryStore.stopLocalModel()
        }

        permissions.objectWillChange
            .merge(with: audio.objectWillChange, screenText.objectWillChange)
            .merge(with: calendar.objectWillChange, meetingTrigger.objectWillChange, googleCalendarPolling.objectWillChange)
            .merge(with: retention.objectWillChange, meetingHeuristicDetector.objectWillChange)
            .merge(with: routing.objectWillChange, syncState.objectWillChange, replyPriority.objectWillChange, meetingPrepSettings.objectWillChange, meetingTitleCheck.objectWillChange, liveTranscriptSettings.objectWillChange, appearanceSettings.objectWillChange)
            .merge(with: liveQuestionSettings.objectWillChange, topicSort.objectWillChange, topicSteps.objectWillChange)
            .merge(with: snapshotSettings.objectWillChange, mediaSettings.objectWillChange, mediaRetention.objectWillChange, recordingCheckInSettings.objectWillChange, awayMode.objectWillChange)
            .merge(with: settingsSyncCoordinator.objectWillChange, contentSyncCoordinator.objectWillChange)
            .merge(with: updateChecker.objectWillChange)
            .merge(with: promptLibrary.objectWillChange, profile.objectWillChange, profileSuggestions.objectWillChange, routineHost.objectWillChange)
            .merge(with: calendarConnectorState.objectWillChange)
            .merge(with: gmailConnectorState.objectWillChange)
            .merge(with: outlook365ConnectorState.objectWillChange)
            .merge(with: retentionScheduler.objectWillChange)
            .merge(with: routineScheduler.objectWillChange)
            .merge(with: mcpServerProcess.objectWillChange)
            .merge(with: hotkeyService.objectWillChange)
            .sink { [weak self] _ in self?.objectWillChange.send() }
            .store(in: &cancellables)

        // Monthly budgets live in core: pushed now, after the store's launch work queued
        // above, and again on every change (a local edit or a sync pull).
        routing.onBudgetsChanged = { [weak self] budgets in self?.pushProviderBudgets(budgets) }
        pushProviderBudgets(routing.allBudgets)

        // Live transcript follows the active meeting: it starts when a recording becomes the
        // active meeting (if the setting is on) and stops when it ends. It reads the recording
        // files lazily, so it does not matter that audio starts before the meeting row exists.
        $activeMeetingId
            .removeDuplicates()
            .sink { [weak self] id in
                guard let self else { return }
                // A recording that ends (or gives way to another) has its waiting questions judged.
                if self.liveQuestionWatcher?.meetingId != id {
                    self.liveQuestionWatcher?.flush()
                    self.liveQuestionWatcher = nil
                }
                if let id, self.liveTranscriptionWanted {
                    self.liveTranscript.start(meetingId: id)
                } else {
                    self.liveTranscript.stop()
                }
            }
            .store(in: &cancellables)

        // The question watcher reads every live tick's rows. It starts on the first rows, by
        // when the meeting has its title.
        liveTranscript.$segments
            .sink { [weak self] rows in self?.feedLiveQuestionWatcher(rows) }
            .store(in: &cancellables)

        // Turning alerts off drops the watcher at once, without judging what is waiting.
        liveQuestionSettings.$enabled
            .removeDuplicates()
            .sink { [weak self] enabled in
                guard !enabled else { return }
                self?.liveQuestionWatcher?.cancel()
                self?.liveQuestionWatcher = nil
            }
            .store(in: &cancellables)

        // Live transcription runs while the live transcript is on, or question alerts are on with a
        // name to listen for (`liveTranscriptionWanted`). These are privacy controls, so they
        // apply at once in either direction: once neither holds during a meeting, live
        // transcription stops and all live text already stored is deleted; once either holds,
        // it starts. (Published values arrive before the properties change, so the rule reads
        // the emitted values, not the settings.)
        liveTranscriptSettings.$enabled
            .combineLatest(liveQuestionSettings.$enabled, profile.$name, profile.$otherNames) { transcript, alerts, name, others in
                AppState.liveTranscriptionWanted(
                    transcript: transcript, alerts: alerts, names: ProfileSettings.names(profileName: name, otherNames: others))
            }
            .removeDuplicates()
            .sink { [weak self] enabled in
                guard let self else { return }
                if enabled {
                    if let id = self.activeMeetingId { self.liveTranscript.start(meetingId: id) }
                } else {
                    self.liveTranscript.stop()
                    // Turned off means gone: delete every provisional live row, not just the
                    // active meeting's (a larger "now" makes every row older than the stale
                    // cutoff).
                    if let store = self.coreStore {
                        CoreMemoryStore.accessQueue.async { store.purgeStaleLiveSegments(nowUnixMs: Int64.max / 2) }
                    }
                }
            }
            .store(in: &cancellables)

        // A privacy control: turning snapshots off mid-meeting stops capture at once; turning
        // them back on resumes it for the same meeting folder.
        snapshotSettings.$enabled
            .removeDuplicates()
            .sink { [weak self] enabled in
                if enabled { self?.meetingSnapshotService?.start() } else { self?.meetingSnapshotService?.stop() }
            }
            .store(in: &cancellables)

        meetingTrigger.onAcceptPrompt = { [weak self] meeting in
            self?.startCapture(for: meeting)
        }
        // The 6 h writer cap or low disk: end the meeting (never just the audio), so the
        // recording is completed and transcribed like any other end. With no meeting row
        // (its insert failed, or hasn't landed yet) there is nothing to end, so stop the
        // audio directly: low disk must never keep writing.
        audio.onForcedStop = { [weak self] stop in
            guard let self else { return }
            let hasMeeting = self.activeMeetingId != nil
            let reason: MeetingEndDetector.EndReason = stop == .durationCap ? .durationCap : .lowDisk
            if hasMeeting {
                self.endActiveMeetingIfNeeded(reason: reason)
            } else {
                self.stopRowlessRecording()
            }
            self.postRecordingNotice(StatusMenuCopy.forcedStop(durationCap: stop == .durationCap, saved: hasMeeting))
        }
        calendar.start()
        googleCalendarPolling.start(calendarConnectorState: calendarConnectorState, coreStore: coreStore)
        gmailPolling.start(gmailConnectorState: gmailConnectorState, coreStore: coreStore, deviceId: deviceId)
        meetingTrigger.start(meetingsProvider: { [weak self] in self?.allUpcomingMeetings ?? [] })
        retentionScheduler.start(retention: retention, media: mediaSettings, deviceId: deviceId, store: coreStore, audio: audio,
                                 runner: mediaRetention)
        // A drive mounting may be the media folder (or hold stored media) coming back.
        mountObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didMountNotification, object: nil, queue: .main
        ) { [weak self] note in
            guard let volume = note.userInfo?[NSWorkspace.volumeURLUserInfoKey] as? URL else { return }
            MainActor.assumeIsolated { self?.mediaRetention.volumeMounted(volume) }
        }
        // A Mac that slept through 15 quiet minutes asks "Still in a meeting?" on wake.
        wakeObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didWakeNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.pollCheckIn() }
        }
        routineScheduler.start(
            store: coreStore, slowStore: slowStore, meetingsProvider: { [weak self] in self?.allUpcomingMeetings ?? [] }, routing: routing,
            promptLibrary: promptLibrary, meetingPrepLeadMinutes: { [weak self] in self?.meetingPrepSettings.leadMinutes ?? 30 },
            calendarConnectorState: calendarConnectorState, gmailConnectorState: gmailConnectorState,
            profile: profile, profileSuggestions: profileSuggestions,
            gate: { [weak self] in
                guard let self else { return .full }
                let approved = Set(self.syncState.devices.filter { $0.status == "approved" }.map(\.publicKey))
                return RoutineHost.gate(primaryId: self.routineHost.primaryDeviceId, ownId: self.deviceId, approvedDeviceIds: approved)
            }
        )
        // A "Prep ready" for briefs another device prepared, once each sync lands.
        if let coreStore {
            contentSyncCoordinator.$lastSyncedAt.dropFirst()
                .receive(on: DispatchQueue.main)
                .sink { [weak self] _ in self?.routineScheduler.announceSyncedPrep(store: coreStore) }
                .store(in: &cancellables)
        }
        Task { await profileSuggestions.reload() }
        Task { await syncState.silentResumeIfPossible() }
        // Ollama's installed models, now and every 10 minutes: they decide which one stands in
        // for a default that isn't installed (`RoutingSettings.ollamaDefaultModel`), with no
        // Settings or picker open. A failed read keeps the last list.
        Task { [routing] in
            while !Task.isCancelled {
                await routing.refreshOllamaModels(showsLoading: false)
                do { try await Task.sleep(for: .seconds(10 * 60)) } catch { break }
            }
        }
        updateChecker.start()
        settingsSyncCoordinator.observeSyncState()
        let clickHandler = NotificationClickHandler(
            open: { [weak self] link in self?.open(link, activateIfUnresolved: true) },
            openPrep: { [weak self] key in self?.showPrepBrief(key: key) },
            openAIProviders: { [weak self] in
                // Set first: SettingsView takes it on appear, or on change when already open.
                self?.pendingSettingsSection = .ai
                NSApp.activate(ignoringOtherApps: true)
                self?.openSettingsWindow?()
            },
            openStorage: { [weak self] in self?.openFreeUpSpace() },
            stillInMeeting: { [weak self] keep in self?.answerStillInMeeting(keep: keep) })
        notificationClickHandler = clickHandler
        UNUserNotificationCenter.current().delegate = clickHandler
        startBudgetNotices()
        contentSyncCoordinator.afterPull = { [weak self] in
            guard let self, let store = self.coreStore else { return }
            await self.liveAlertDelivery.afterPull {
                await AppState.run(on: CoreMemoryStore.accessQueue) {
                    store.takeUnnotifiedPeerAlerts(nowUnixMs: Int64(Date().timeIntervalSince1970 * 1000), maxAgeMs: LiveAlertDelivery.peerMaxAgeMs)
                }
            }
        }
        contentSyncCoordinator.observeSyncState()
        syncState.onSyncReset = { [weak self] in self?.contentSyncCoordinator.forgetSyncHistory() }
        scheduleJobsBatchStep()
        scheduleEpisodeSegmentStep()
        scheduleReplyPriorityStep()
        scheduleTopicsStep()
        scheduleTopicVaultStep(after: 60)
        scheduleTopicStepsStep()
        scheduleTopicNamingStep()
        scheduleScreenTextIngestStep()
        scheduleTranscriptBackfillStep()

        // Read the calendar the moment access is granted, not at the next 60 s refresh.
        permissions.$calendarGranted
            .removeDuplicates()
            .dropFirst()
            .sink { [weak self] granted in
                if granted { self?.calendar.accessGranted() }
            }
            .store(in: &cancellables)

        // FR-CAP-13: activity capture starts automatically the first time Accessibility
        // permission is confirmed granted -- during onboarding, or later via Settings --
        // removing the need for a separate manual step. `@Published`'s projected
        // publisher emits the CURRENT value immediately on subscribe, so if permission
        // was already granted before this app launch, this also covers that case (fires
        // right here at startup). `removeDuplicates()` prevents re-firing on every
        // subsequent `refresh()` poll tick once the value has settled. If the user
        // already toggled activity capture on before permission was granted (the app
        // allows this -- FR-ONB-2, runs under partial grants), `screenText` itself never
        // actually started; this handles that case by starting it directly once granted,
        // rather than going through `resumeActivityCapture()` (which would no-op since
        // `activityCaptureEnabled` is already true).
        permissions.$accessibilityGranted
            .removeDuplicates()
            .sink { [weak self] granted in
                guard let self, granted else { return }
                if self.activityCaptureEnabled {
                    self.screenText.start()
                } else {
                    self.resumeActivityCapture(accessibilityGranted: granted)
                }
            }
            .store(in: &cancellables)

        // FR-MTG-2: a detected meeting only gets handled while no meeting is already
        // active -- if one's already running (manual, calendar, or a previous
        // detection), there's nothing useful to prompt for. Independent of activity
        // capture's state (FR-MTG-14) -- a detected meeting can be handled whether
        // activity capture is on or paused.
        meetingHeuristicDetector.$detectedMeeting
            .compactMap { $0 }
            .sink { [weak self] meeting in
                guard let self else { return }
                // An Away Mode join holds the mic before its capture starts: not a meeting to prompt for.
                // Nor while a start is in flight (a split's next meeting: its call is still on).
                if self.activeMeetingId == nil, !self.isStartingMeeting, !self.awayCoordinator.isRunning {
                    self.meetingTrigger.handleDetectedMeeting(meeting)
                }
                self.meetingHeuristicDetector.clearDetection()
            }
            .store(in: &cancellables)

        // Gate the whole poller (not just its browser-URL-reading branch) on the
        // Settings toggle — `@Published`'s projected publisher emits the current value
        // immediately on subscribe, so this also covers app-launch: if the setting is
        // already off when this sink is set up, `stop()` fires right here with no extra
        // code needed for that case. `enabled` is the sink's own parameter, not a fresh
        // read of `meetingTrigger.heuristicDetectionEnabled` (which `@Published` updates
        // in `willSet`, i.e. still the *old* value while this sink runs).
        meetingTrigger.$heuristicDetectionEnabled
            .sink { [weak self] enabled in
                guard let self else { return }
                if enabled {
                    self.meetingHeuristicDetector.start()
                } else {
                    self.meetingHeuristicDetector.stop()
                }
            }
            .store(in: &cancellables)

        // Task 7: start/stop the real MCP server process from the "Enable MCP Server"
        // Settings toggle -- `@Published`'s projected publisher emits the current value
        // immediately on subscribe, so this also covers app launch: if the toggle was
        // left on from a previous session, the server resumes right here with no extra
        // code needed for that case (same reasoning as the `heuristicDetectionEnabled`
        // sink above). Explicit toggle-off and full app quit are both handled by
        // `McpServerProcessManager` itself (`stop()`, and its own
        // `NSApplication.willTerminateNotification` observer, respectively).
        mcpServerProcess.$enabled
            .sink { [weak self] enabled in
                guard let self else { return }
                if enabled {
                    self.mcpServerProcess.start()
                } else {
                    self.mcpServerProcess.stop()
                }
            }
            .store(in: &cancellables)

        frontmostTracker.start()
        // The frontmost app is read first, before the panel can take focus.
        onHotkey = { [weak self] in
            let frontmost = NSWorkspace.shared.frontmostApplication.flatMap(FrontmostAppTracker.frontmost)
            self?.toggleHotkeyPanel(frontmost: frontmost)
        }
        hotkeyService.start { [weak self] in self?.onHotkey?() }

        meetingTrigger.$pendingPrompt
            .sink { [weak self] prompt in
                guard let self else { return }
                guard let prompt else {
                    self.callWaitTimer?.invalidate()
                    self.callWaitTimer = nil
                    self.meetingAlertPanel.hidePrompt()
                    return
                }
                self.handlePrompt(prompt)
            }
            .store(in: &cancellables)

        // `@Published` fires on willSet, so read the sources after the hop (as `persistCalendarEventItems` does).
        calendar.$upcomingWeek
            .merge(with: googleCalendarPolling.$oauthMeetings)
            .sink { [weak self] _ in
                DispatchQueue.main.async {
                    guard let self else { return }
                    self.upcomingWeekMeetings = Self.weekMeetings(
                        eventKit: self.calendar.upcomingWeek, oauth: self.googleCalendarPolling.oauthMeetings, now: Date()
                    )
                }
            }
            .store(in: &cancellables)

        calendar.$upcomingMeetings
            .merge(with: calendar.$upcomingWeek)
            .merge(with: googleCalendarPolling.$oauthMeetings)
            .sink { [weak self] _ in
                DispatchQueue.main.async { self?.persistCalendarEventItems() }
            }
            .store(in: &cancellables)
    }

    /// A new pending prompt: Away Mode's if it takes it, else the alert panel (or a hold).
    /// Also where a held prompt goes once its recording ends on its own.
    private func handlePrompt(_ prompt: QualifyingMeeting) {
        callWaitTimer?.invalidate()
        callWaitTimer = nil
        if AwayModeCoordinator.takesPrompt(prompt, enabled: awayMode.enabled, joinAvailable: Self.awayZoomJoinEnabled) {
            meetingAlertPanel.hidePrompt()
            scheduleAwayRun(for: prompt)
            return
        }
        promptNormally(prompt)
    }

    /// The meeting-alert panel for a pending prompt (Away Mode off, or it handed the meeting back).
    private func promptNormally(_ prompt: QualifyingMeeting) {
        // During a recording the prompt couldn't start anything (`startCapture` needs no
        // active meeting), so a calendar prompt waits for its start and splits instead.
        if prompt.source != .detected, activeMeetingId != nil || isStartingMeeting {
            holdForSplit(prompt)
            return
        }
        // A calendar event only auto-starts once this Mac is in the call (mic and audio
        // output both in use): with several Macs in one room, an event on a Mac that
        // isn't in it would otherwise record whatever its mic overhears. Until then the
        // prompt waits, and the countdown starts if the call begins while it's showing.
        let decision = CallAutoStart.decide(autoStartEnabled: self.meetingTrigger.autoStartEnabled,
                                            source: prompt.source, inCall: CallAutoStart.isInCall())
        self.showMeetingAlert(for: prompt, autoStart: decision.autoStart)
        guard decision.waitForCall else { return }
        self.callWaitTimer = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] timer in
            Task { @MainActor in
                guard let self, self.meetingTrigger.pendingPrompt?.id == prompt.id else {
                    timer.invalidate()
                    return
                }
                guard CallAutoStart.isInCall() else { return }
                timer.invalidate()
                self.callWaitTimer = nil
                self.showMeetingAlert(for: prompt, autoStart: true)
            }
        }
    }

    /// The recording `MeetingSplit` compares a next meeting against; nil until its row lands.
    /// Its start is the meeting's original one, so a continued or joined meeting counts from
    /// when it really began.
    private var splitCurrent: MeetingSplit.Current? {
        guard activeMeetingId != nil, let activeMeetingStartedAt else { return nil }
        return .init(calendarEvent: activeCalendarEvent, link: activeCallLink, meetingStartedAt: activeMeetingStartedAt)
    }

    /// The call's current URL where it can be read: a Google Meet tab in the frontmost browser.
    // ponytail: native Zoom/Teams expose no meeting ID here, so rule 2 only sees Meet tabs.
    private func liveCallURL() -> String? {
        MeetingHeuristicDetector.resolveFrontmostGoogleMeetWindow().flatMap(BrowserMeetingCheck.currentURL(of:))
    }

    /// Holds a calendar prompt that came in during a recording, checking every 5 s
    /// (`MeetingSplit.holdAction`, with the event re-read from the calendar) until its
    /// meeting starts and splits, the recording ends on its own (then it's a normal prompt),
    /// or the meeting is over, gone or declined.
    private func holdForSplit(_ next: QualifyingMeeting) {
        Task { @MainActor [weak self] in
            while true {
                guard let self, self.meetingTrigger.pendingPrompt?.id == next.id else { return }
                // An Away Mode recording is never split (it leaves the call itself): wait.
                let action = MeetingSplit.holdAction(
                    recording: self.activeMeetingId != nil || self.isStartingMeeting,
                    current: self.awayCoordinator.isRunning ? nil : self.splitCurrent,
                    next: self.allUpcomingMeetings.first { $0.id == next.id },
                    liveCallURL: { self.liveCallURL() }, now: Date()
                )
                switch action {
                case .wait: break
                case .prompt: return self.handlePrompt(next)
                case .split: return self.splitForNextMeeting(next)
                case .drop: return self.meetingTrigger.dismissPrompt()
                }
                try? await Task.sleep(nanoseconds: 5_000_000_000)
            }
        }
    }

    /// A calendar prompt still showing when another recording starts could no longer start
    /// its meeting (`startCapture`'s guard), so it is held for a split too. `startingId` is
    /// the meeting being started, whose own prompt (an Away Mode run's) is left alone.
    private func holdPendingPromptForSplit(startingId: String?) {
        guard let prompt = meetingTrigger.pendingPrompt, prompt.source != .detected, prompt.id != startingId else { return }
        callWaitTimer?.invalidate()
        callWaitTimer = nil
        meetingAlertPanel.hidePrompt()
        holdForSplit(prompt)
    }

    /// Ends the active meeting and starts `next` (still the pending prompt), then offers
    /// "Keep together". Audio stops and restarts, a sub-second gap between the two files.
    private func splitForNextMeeting(_ next: QualifyingMeeting) {
        guard let earlierId = activeMeetingId else { return }
        // Low disk: the next meeting couldn't start, so keep recording this one whole
        // (`hasRoomToRecord` posts the usual notice) and drop the held prompt.
        guard hasRoomToRecord() else {
            meetingTrigger.dismissPrompt()
            return
        }
        let earlierTitle = activeMeetingTitle ?? "Meeting"
        MeetingSplit.log.notice("meeting \(earlierId): split for the next calendar meeting")
        endMeeting(reason: .split)
        let splitAt = Date()
        meetingTrigger.acceptPrompt()  // -> startCapture(for: next); hides the prompt panel
        guard isStartingMeeting else { return }  // refused (disk): nothing to keep together
        let generation = captureGeneration
        meetingAlertPanel.showKeepTogether(for: next, earlierTitle: earlierTitle) { [weak self] in
            self?.keepTogether(earlierId: earlierId, splitAt: splitAt, generation: generation)
        }
        // Offered for as long as meeting continue would have joined a restart.
        DispatchQueue.main.asyncAfter(deadline: .now() + MeetingContinuation.continueWindow) { [weak self] in
            self?.meetingAlertPanel.hideKeepTogether(for: next.id)
        }
    }

    /// "Keep together": the split-off meeting's recording becomes the earlier meeting's next
    /// part, as meeting continue does, and the split-off meeting is deleted. Audio keeps
    /// going; only the bookkeeping moves. `generation` is the split's start: if that
    /// recording ended or was replaced meanwhile, nothing changes (or the reopen is undone).
    /// The rejoin queues behind the split-off meeting's own insert on `accessQueue`, so a tap
    /// before that row lands still works.
    private func keepTogether(earlierId: Int64, splitAt: Date, generation: Int) {
        guard generation == captureGeneration, Date().timeIntervalSince(splitAt) <= MeetingContinuation.continueWindow,
              let store = coreStore else { return }
        let partStartMs = Int64(splitAt.timeIntervalSince1970 * 1000)
        CoreMemoryStore.accessQueue.async {
            guard let rejoined = MeetingSplit.rejoin(store: store, meetingId: earlierId, partStartUnixMs: partStartMs) else {
                MeetingSplit.log.error("meeting \(earlierId): keep together could not add a part; left split")
                return
            }
            DispatchQueue.main.async { [weak self] in
                guard let self, generation == self.captureGeneration, !self.isStartingMeeting,
                      let splitId = self.activeMeetingId, splitId != earlierId else {
                    MeetingSplit.log.error("meeting \(earlierId): keep together lost to the recording ending; left split")
                    CoreMemoryStore.accessQueue.async { store.undoMeetingReopen(meetingId: earlierId, partIndex: rejoined.partIndex) }
                    return
                }
                self.finishKeepTogether(rejoined, splitId: splitId, store: store)
            }
        }
    }

    private func finishKeepTogether(_ rejoined: MeetingSplit.Rejoined, splitId: Int64, store: CoreMemoryStore) {
        let earlier = rejoined.meeting
        // Stop the split-off meeting's snapshots before its slides move.
        meetingSnapshotService?.stop()
        meetingSnapshotService = nil
        // Queued before `activeMeetingId` changes, so the live writes for the joined part land after.
        CoreMemoryStore.accessQueue.async { store.clearLiveSegments(meetingId: earlier.id) }
        let start = Date(timeIntervalSince1970: TimeInterval(earlier.startUnixMs) / 1000)
        let typedSinceSplit = meetingNotes
        let carried = MeetingContinuation.notes(stored: earlier.notes, lastEnded: lastEndedMeetingNotes, meetingId: earlier.id)
        meetingNotes = [carried, typedSinceSplit].filter { !$0.isEmpty }.joined(separator: "\n")
        activeMeetingId = earlier.id
        activeMeetingStartedAt = start
        activeMeetingTitle = earlier.title
        activeMeetingIsContinued = true
        activePartIndex = rejoined.partIndex
        // The event, end detector and check-in stay the split-off meeting's: they describe
        // the call now recording.
        // The split-off slides move into the earlier meeting (never deleted), then its row
        // goes. Behind the media retention work in flight, like the manual split. Snapshots
        // restart once the earlier meeting's folder is settled.
        let storage = makeSnapshotStorage()
        let appBundleID = activeMeetingBundleID, provider = activeCallLink?.provider
        MediaMover.queue.async {
            let dir: String? = CoreMemoryStore.accessQueue.sync {
                switch MeetingSplit.absorb(store: store, into: earlier.id, from: splitId,
                                           makeFolder: { try storage.makeMeetingFolder(start: $0, title: $1) }) {
                case nil:
                    MeetingSplit.log.error("meeting \(splitId): keep together could not move its slides; row kept")
                case .some:
                    MeetingSplit.discard(store: store, meetingId: splitId)
                }
                return store.meetingSnapshotsInfo(meetingId: earlier.id)?.dir
            }
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                MeetingLifecycle.postMeetingsChanged([earlier.id, splitId])
                guard self.activeMeetingId == earlier.id else { return }
                let existingFolder = dir
                    .map { URL(fileURLWithPath: $0, isDirectory: true) }
                    .flatMap { FileManager.default.fileExists(atPath: $0.path) ? $0 : nil }
                self.startMeetingSnapshots(meetingId: earlier.id, start: start, title: earlier.title, appBundleID: appBundleID,
                                           provider: provider, existingFolder: existingFolder, browserWindowProvider: { nil })
            }
        }
    }

    /// docs/superpowers/specs/2026-08-07-items-schema-migration-design.md: persists
    /// calendar events into `items` whenever the merged calendar/Google Calendar poll
    /// changes — one integration point regardless of which calendar an event came from.
    /// `$upcomingMeetings`/`$oauthMeetings`, like `objectWillChange`, fire on `willSet` —
    /// *before* the new value lands — so reading the sources synchronously inside the
    /// sink would see the previous poll's data. The subscription above defers
    /// the call with `DispatchQueue.main.async` so it runs after both `@Published`
    /// properties have finished mutating, and the sources reflect the fresh
    /// poll by the time this reads them (what is synced is `syncMeetings`, not
    /// `allUpcomingMeetings`). The actual FFI call is dispatched onto
    /// `CoreMemoryStore.accessQueue` — the same shared queue every other coreStore
    /// caller in this file uses, per `CoreMemoryStore`'s not-thread-safe contract.
    private var lastPresenceCount: Int?
    private var presenceEmptyStreak = 0
    private var lastPresenceGeneration = 0

    private func persistCalendarEventItems() {
        let now = Date()
        let meetings = Self.syncMeetings(
            eventKit: calendar.upcomingMeetings, week: calendar.upcomingWeek, oauth: googleCalendarPolling.oauthMeetings, now: now
        )
        let inputs = meetings.compactMap { CoreMemoryStore.calendarEventItem(for: $0, deviceId: deviceId) }
        let confirmed = Self.confirmedPresence(
            current: Self.eventKitPresence(
                eventKit: calendar.upcomingMeetings, week: calendar.upcomingWeek, now: now, fetchSucceeded: calendar.lastFetchSucceeded
            ),
            previousCount: lastPresenceCount, emptyStreak: presenceEmptyStreak,
            generation: calendar.fetchGeneration, lastGeneration: lastPresenceGeneration
        )
        presenceEmptyStreak = confirmed.emptyStreak
        lastPresenceGeneration = confirmed.lastGeneration
        let present = confirmed.present
        if let present { lastPresenceCount = present.count }
        guard let store = coreStore, !inputs.isEmpty || present != nil else { return }
        CoreMemoryStore.accessQueue.async {
            store.upsertCalendarEventItems(inputs)
            // After the upsert, so a meeting never flickers absent. An empty list is a real answer.
            if let present {
                store.calendarPresenceReplace(
                    source: "apple_calendar", account: "eventkit",
                    windowStart: now, windowEnd: now.addingTimeInterval(Self.calendarSyncHorizon), present: present
                )
            }
        }
    }

    var statusSummary: String {
        var parts: [String] = []
        if audio.systemAudioActive { parts.append("system audio") }
        if audio.micActive { parts.append("mic") }
        if screenText.isRunning { parts.append("screen text") }
        let suffix: String
        if parts.isEmpty {
            suffix = activityCaptureEnabled ? String(localized: "Activity capture on — waiting for permissions") : String(localized: "Activity capture paused")
        } else {
            suffix = "Capturing: \(parts.joined(separator: ", "))"
        }
        guard let activeMeetingTitle else { return suffix }
        return "\(suffix) — \(activeMeetingTitle)"
    }

    /// FR-ONB-2: capability -> reason it's currently inactive.
    var inactiveCapabilities: [(capability: String, reason: String)] {
        var reasons: [(String, String)] = []
        if !permissions.screenRecordingGranted {
            reasons.append(("System audio", "Screen & System Audio Recording permission not granted."))
        }
        if !permissions.microphoneGranted {
            reasons.append(("Microphone", "Microphone permission not granted."))
        }
        if !permissions.accessibilityGranted {
            reasons.append(("Screen text", "Accessibility permission not granted."))
        }
        if activeMeetingId != nil, let degradedReason = audio.degradedReason {
            reasons.append(("System audio", degradedReason))
        }
        if activeMeetingId != nil, let micDegradedReason = audio.micDegradedReason {
            reasons.append(("Microphone", micDegradedReason))
        }
        return reasons
    }

    func toggleActivityCapture() {
        if activityCaptureEnabled {
            pauseActivityCapture()
        } else {
            resumeActivityCapture()
        }
    }

    /// FR-CAP-13: activity capture is independent of meeting state in both
    /// directions -- this never touches `activeMeetingId`/audio.
    private func resumeActivityCapture(accessibilityGranted: Bool? = nil) {
        guard !activityCaptureEnabled else { return }
        activityCaptureEnabled = true
        if accessibilityGranted ?? permissions.accessibilityGranted {
            screenText.start()
        }
    }

    private func pauseActivityCapture() {
        activityCaptureEnabled = false
        screenText.stop()
    }

    /// FR-MTG-5: the single entry point for ending the active meeting — used by both
    /// the manual "End Meeting" button and `MeetingEndDetector`'s automatic heuristics.
    /// With no meeting row (e.g. the DB insert for a manual capture failed, leaving
    /// `activeMeetingId` nil per `startManualMeeting()`'s degrade-gracefully behavior)
    /// but audio still recording, it stops the recording rather than no-op; with an
    /// insert still in flight it waits for that. Independent of activity capture
    /// (FR-MTG-14) -- this never touches `activityCaptureEnabled`/`screenText`.
    /// `reason` is set only by `MeetingEndDetector`; a manual End passes none.
    func endActiveMeetingIfNeeded(reason: MeetingEndDetector.EndReason? = nil) {
        if activeMeetingId != nil {
            endMeeting(reason: reason)
        } else if !isStartingMeeting, audio.systemAudioActive || audio.micActive || meetingEndDetector.isRunning {
            stopRowlessRecording()
        }
    }

    /// Stops a recording that has no meeting row (its insert failed, or is still in flight)
    /// and clears its state. Bumping the generation abandons an in-flight start, so its row
    /// completes instead of going live. The audio stays on disk; the launch re-link can't
    /// match it (no row), so retention's orphan sweep is what eventually removes it.
    private func stopRowlessRecording() {
        captureGeneration += 1
        audio.stop()
        meetingEndDetector.stop()
        stopCheckIn()
        participantCountEstimator?.stop()
        participantCountEstimator = nil
        if let bundleID = activeMeetingBundleID {
            meetingHeuristicDetector.meetingEnded(bundleID: bundleID)
        }
        activeMeetingBundleID = nil
        activeCalendarEvent = nil
        activeCallLink = nil
        activeMeetingTitle = nil
        activeMeetingStartedAt = nil
        activePartIndex = nil
        activeMeetingIsContinued = false
    }

    /// FR-MTG-14: an explicit "Start Meeting" action, independent of activity
    /// capture's state in either direction. Gives a manual meeting a real `meeting`
    /// row, mirroring `startCapture(for:)`'s async-insert-then-proceed pattern — but
    /// without a `QualifyingMeeting`, since a manual meeting has no calendar event or
    /// detected-app context to build one from.
    func startManualMeeting() {
        guard activeMeetingId == nil, !isStartingMeeting, hasRoomToRecord() else { return }
        permissions.request(.notifications) // no-op once answered
        // Set immediately (synchronously, before the async insertMeeting round-trip
        // below) so a rapid second "Start Meeting" click during that gap is blocked —
        // `activeMeetingId` itself isn't set until the round-trip completes. Mirrors
        // `startCapture(for:)`'s identical synchronous set for the same reason.
        isStartingMeeting = true
        holdPendingPromptForSplit(startingId: nil)
        captureGeneration += 1
        let thisGeneration = captureGeneration
        let startDate = Date()
        let startMs = Int64(startDate.timeIntervalSince1970 * 1000)
        let newMeeting = CoreNewMeeting(
            deviceId: deviceId,
            startUnixMs: startMs,
            endUnixMs: nil,
            title: "Meeting",
            attendees: [],
            trigger: .manual,
            audioPath: nil,
            status: .recording
        )
        guard let store = coreStore else {
            finishStartingManualCapture(insertedMeetingId: nil, partIndex: nil, start: startDate, trigger: newMeeting.trigger, generation: thisGeneration)
            return
        }
        CoreMemoryStore.accessQueue.async {
            let insertedId = store.insertMeeting(newMeeting)
            // Manual meetings have no continue key, so they never continue.
            let partIndex = insertedId.flatMap { store.beginMeetingPart(meetingId: $0, startUnixMs: startMs, continueKey: nil) }
            DispatchQueue.main.async { [weak self] in
                self?.finishStartingManualCapture(insertedMeetingId: insertedId, partIndex: partIndex, start: startDate, trigger: newMeeting.trigger, generation: thisGeneration)
            }
        }
    }

    /// Mirrors `finishStartingCapture(for:insertedMeetingId:generation:)`'s exact
    /// race-guard shape: if a newer meeting-start happened during the async insert
    /// above (`generation != captureGeneration`), the just-inserted row (if any) is
    /// immediately completed with a null `audioPath` — no audio ever started for that
    /// already-cancelled session — rather than left stuck at `.recording` forever.
    private func finishStartingManualCapture(insertedMeetingId: Int64?, partIndex: Int64?, start: Date, trigger: CoreMeetingTrigger, generation: Int) {
        isStartingMeeting = false
        guard generation == captureGeneration else {
            completeAbandonedStart(meetingId: insertedMeetingId, partIndex: partIndex)
            return
        }
        activeMeetingId = insertedMeetingId
        activeMeetingStartedAt = insertedMeetingId == nil ? nil : start
        activeMeetingTitle = "Meeting"
        activeMeetingIsContinued = false
        activePartIndex = partIndex
        activeMeetingBundleID = nil
        activeCalendarEvent = nil
        activeCallLink = nil
        meetingNotes = ""
        Task { await audio.start() }
        if let insertedMeetingId {
            startMeetingSnapshots(meetingId: insertedMeetingId, start: start, title: "Meeting",
                                  appBundleID: nil, provider: nil, browserWindowProvider: { nil })
            startCheckIn(startedAt: start)
            if StillInMeeting.applies(to: trigger) { stillInMeeting = (promptShownAt: nil, keptAt: nil, polledAt: nil) }
        }
        // No app, tab or schedule, and audio always "active" so a quiet room never trips the
        // silence rule: only max duration (4 h, core) ends a manual meeting here.
        meetingEndDetector.start(
            scheduledEndUnixMs: nil,
            appBundleID: nil,
            browserMeetingWindow: nil,
            isAudioActive: { true },
            suppressMaxDuration: checkInSuppressesMaxDuration,
            onEnded: { [weak self] reason in self?.endActiveMeetingIfNeeded(reason: reason) }
        )
    }

    /// Disk preflight (spec §4): below 5 GB free, refuse to start. Runs before any start
    /// state is set, so a refusal leaves nothing to undo.
    /// Sets `menuBarElapsed` now and every 30 s while a meeting records; clears it when none does.
    /// Publishes only when the text changes, so the menu bar redraws at most once a minute.
    private func restartMenuBarElapsedTimer() {
        menuBarElapsedTimer?.invalidate()
        menuBarElapsedTimer = nil
        guard activeMeetingStartedAt != nil else {
            menuBarElapsed = nil
            return
        }
        refreshMenuBarElapsed()
        let timer = Timer(timeInterval: 30, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refreshMenuBarElapsed() }
        }
        RunLoop.main.add(timer, forMode: .common)
        menuBarElapsedTimer = timer
    }

    private func refreshMenuBarElapsed() {
        guard let startedAt = activeMeetingStartedAt else { return }
        let text = ElapsedLabel.text(Date().timeIntervalSince(startedAt))
        if text != menuBarElapsed { menuBarElapsed = text }
    }

    private func hasRoomToRecord() -> Bool {
        guard RecordingLimits.canStart(freeBytes: RecordingLimits.freeBytes()) else {
            postRecordingNotice(StatusMenuCopy.lowDiskStart)
            return false
        }
        return true
    }

    /// A plain notification (no actions: the app has no notification delegate).
    private func postRecordingNotice(_ body: String) {
        let content = UNMutableNotificationContent()
        content.title = "Whimbrel"
        content.body = body
        content.sound = .default
        let request = UNNotificationRequest(identifier: "recording-notice-\(UUID().uuidString)", content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request) { error in
            if let error {
                Logger(subsystem: "com.mpointer.kestrel", category: "recording-failsafe")
                    .error("Failed to post recording notice: \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    /// Passed to both detector starts: a showing prompt or a recent "Keep recording"
    /// holds off the 4 h max duration (ruling R0/R5).
    private var checkInSuppressesMaxDuration: () -> Bool {
        { [weak self] in self?.activeCheckIn?.suppressesMaxDuration(now: Date()) ?? false }
    }

    /// The "Still recording?" check-in (spec §2). `startedAt` is the meeting's start, the
    /// continued meeting's original one for a continuation. The 5 s poll decides; the
    /// panel and notification only display.
    private func startCheckIn(startedAt: Date) {
        stopCheckIn()
        activeCheckIn = recordingCheckInSettings.makeCheckIn(startedAt: startedAt)
        _ = audio.micPeak.take(); _ = audio.systemPeak.take()   // drop stale peaks
        silenceWatch = SilenceWatch()
        let timer = Timer(timeInterval: 5, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.pollCheckIn() }
        }
        RunLoop.main.add(timer, forMode: .common)
        checkInTimer = timer
    }

    private func stopCheckIn() {
        checkInTimer?.invalidate()
        checkInTimer = nil
        activeCheckIn = nil
        silenceWatch = nil
        silenceWarning = nil
        recordingCheckInPanel.hide()
        if stillInMeeting != nil { removeStillInMeetingNotice() }
        stillInMeeting = nil
    }

    private func pollSilence() {
        guard var watch = silenceWatch else { return }
        let mic = SilenceWatch.decibels(fromPeak: audio.micPeak.take())
        let system = SilenceWatch.decibels(fromPeak: audio.systemPeak.take())
        // No system track (Room audio, or system capture degraded): the mic alone decides.
        let event = watch.feed(at: Date(), systemDB: audio.systemAudioActive ? system : nil, micDB: mic)
        silenceWatch = watch
        switch event {
        case .warn:
            let title = activeMeetingTitle ?? "Meeting"
            let body = String(localized: "\(title) has been silent for 3 minutes. Check that the meeting has started and your audio devices are right.")
            silenceWarning = body
            guard let id = activeMeetingId else { return }
            LiveAlertDelivery.postToNotificationCenter(.init(
                id: "silence-warning-\(id)", title: String(localized: "Whimbrel isn't hearing anything"), body: body,
                deepLink: DeepLink.meetingURL(id: id, atCs: 0)))
        case .clear:
            silenceWarning = nil
        case nil:
            break
        }
    }

    private func pollCheckIn() {
        pollSilence()
        // Both due: "Still in a meeting?" shows and the hourly check-in waits (it is time-based,
        // so it only defers). An hourly prompt already showing goes first.
        if activeCheckIn?.promptShownAt == nil, pollStillInMeeting() { return }
        guard var checkIn = activeCheckIn else { return }
        let action = checkIn.poll(now: Date())
        activeCheckIn = checkIn
        switch action {
        case .none:
            break
        case .showPrompt(let deadline):
            recordingCheckInPanel.show(
                deadline: deadline,
                onKeep: { [weak self] in self?.activeCheckIn?.keepRecording(now: Date()) },
                onStop: { [weak self] in self?.endActiveMeetingIfNeeded(reason: .checkInStop) }
            )
            postRecordingNotice(StatusMenuCopy.checkInWarning)
        case .stop:
            recordingCheckInPanel.hide()
            endActiveMeetingIfNeeded(reason: .checkInTimeout)
        }
    }

    /// True while its prompt shows or it stopped the recording.
    private func pollStillInMeeting() -> Bool {
        guard let watch = stillInMeeting, let startedAt = activeMeetingStartedAt, activeMeetingId != nil else { return false }
        let now = Date()
        // A prompt whose answer window ran out while the Mac slept was never seen: ask again.
        let shown = StillInMeeting.promptShownAt(now: now, lastPollAt: watch.polledAt, promptShownAt: watch.promptShownAt)
        stillInMeeting?.polledAt = now
        if let shown, shown != watch.promptShownAt {
            stillInMeeting?.promptShownAt = shown
            showStillInMeetingPrompt(now: now)
        }
        // Late (backlogged) live lines count as speech when they arrive; off live, nothing does.
        let arrived = liveTranscript.status == .listening ? liveTranscript.lastSpeechArrivedAt : nil
        let action = StillInMeeting.state(now: now, recordingStartedAt: startedAt, lastSpeechAt: lastSpeechAt(startedAt: startedAt),
                                          speechArrivedAt: arrived, promptShownAt: shown, keptAt: watch.keptAt)
        switch action {
        case .none:
            if shown != nil {   // someone spoke while it asked
                stillInMeeting?.promptShownAt = nil
                recordingCheckInPanel.hide()
                removeStillInMeetingNotice()
            }
            return false
        case .prompt:
            guard shown == nil else { return true }
            stillInMeeting?.promptShownAt = now
            showStillInMeetingPrompt(now: now)
            return true
        case .stop:
            stopQuietRecording(reason: .checkInTimeout)
            return true
        }
    }

    private func showStillInMeetingPrompt(now: Date) {
        recordingCheckInPanel.show(
            deadline: now.addingTimeInterval(Double(StillInMeeting.answerSeconds)), stillInMeeting: true,
            onKeep: { [weak self] in self?.answerStillInMeeting(keep: true) },
            onStop: { [weak self] in self?.answerStillInMeeting(keep: false) })
        let content = UNMutableNotificationContent()
        content.title = String(localized: "Still in a meeting?")
        content.body = String(localized: "No one has spoken for 15 minutes.")
        content.sound = .default
        content.categoryIdentifier = StillInMeeting.notificationCategory
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: Self.stillInMeetingNoticeId, content: content, trigger: nil))
    }

    private func lastSpeechAt(startedAt: Date) -> Date? {
        StillInMeeting.lastSpeechAt(recordingStartedAt: startedAt, liveListening: liveTranscript.status == .listening,
                                    liveSegments: liveTranscript.segments, lastNonSilentAt: audio.lastNonSilentAt)
    }

    /// The panel's or the notification's button. Only while its prompt shows, so a stale
    /// notification can't touch the hourly prompt or a later recording.
    private func answerStillInMeeting(keep: Bool) {
        guard stillInMeeting?.promptShownAt != nil else { return }
        if keep {
            stillInMeeting?.promptShownAt = nil
            stillInMeeting?.keptAt = Date()   // re-arms only this timer
            recordingCheckInPanel.hide()
            removeStillInMeetingNotice()
        } else {
            stopQuietRecording(reason: .checkInStop)
        }
    }

    /// Stops untrimmed; the transcription queue trims it once the full transcript is stored.
    private func stopQuietRecording(reason: MeetingEndDetector.EndReason) {
        guard activeMeetingId != nil else { return }
        endMeeting(reason: reason, autoTrim: true)   // stopCheckIn hides the panel and notice
    }

    private static let stillInMeetingNoticeId = "still-in-meeting"

    private func removeStillInMeetingNotice() {
        UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [Self.stillInMeetingNoticeId])
    }

    /// Snapshots go to `<media>/Snapshots`, falling back to the data folder when the media folder
    /// is missing (`SnapshotStorage` only uses a root whose parent exists). The default
    /// `~/Pictures/Whimbrel` is created here so first use doesn't fall back; a chosen folder
    /// is never created, since it may be an unmounted drive.
    private func makeSnapshotStorage() -> SnapshotStorage {
        if mediaSettings.folderPath == nil {
            try? FileManager.default.createDirectory(at: mediaSettings.folderURL, withIntermediateDirectories: true)
        }
        return SnapshotStorage(root: mediaSettings.snapshotsURL,
                               fallbackRoot: SpikePaths.root.appendingPathComponent("Snapshots"))
    }

    /// Calendar, detected and manual meetings alike; a nil `appBundleID` makes the source pick
    /// the meeting window per tick (`WindowFrameSource.resolveTarget`). Stopped in `endMeeting`.
    private func startMeetingSnapshots(meetingId: Int64, start: Date, title: String, appBundleID: String?,
                                       provider: MeetingProvider?, existingFolder: URL? = nil,
                                       browserWindowProvider: @escaping () -> AXUIElement?) {
        guard let store = coreStore else { return }
        meetingSnapshotService?.stop()
        meetingSnapshotService = MeetingSnapshotService(
            meetingId: meetingId, meetingStart: start, meetingTitle: title,
            source: WindowFrameSource(appBundleID: appBundleID, provider: provider,
                                      browserWindowProvider: browserWindowProvider),
            storage: makeSnapshotStorage(),
            store: store, existingFolder: existingFolder
        )
        if snapshotSettings.enabled { meetingSnapshotService?.start() }
    }

    /// FR-MTG-14: ends the active meeting -- audio + completion/transcription only,
    /// independent of activity capture (never touches `activityCaptureEnabled`/
    /// `screenText`).
    /// `autoTrim` ("Still in a meeting?"): the transcription job trims the meeting back to its
    /// last stored speech once the full transcript is stored, before naming and the summary.
    private func endMeeting(reason: MeetingEndDetector.EndReason?, autoTrim: Bool = false) {
        captureGeneration += 1
        audio.stop()
        meetingEndDetector.stop()
        stopCheckIn()
        participantCountEstimator?.stop()
        participantCountEstimator = nil
        meetingSnapshotService?.stop()
        meetingSnapshotService = nil
        if let bundleID = activeMeetingBundleID {
            meetingHeuristicDetector.meetingEnded(bundleID: bundleID)
        }
        activeMeetingBundleID = nil
        if let event = activeCalendarEvent {
            meetingTrigger.meetingEnded(eventId: event.id, reason: reason)
            if let id = activeMeetingId {
                lastEndedCalendarMeeting = .init(meetingId: id, event: event, endedAt: Date())
            }
        }
        activeCalendarEvent = nil
        activeCallLink = nil
        meetingAlertPanel.hideKeepTogether()
        let partIndex = activePartIndex
        let continued = activeMeetingIsContinued
        activePartIndex = nil
        activeMeetingIsContinued = false
        if let id = activeMeetingId {
            let endMs = Int64(Date().timeIntervalSince1970 * 1000)
            let audioPath = audio.systemAudioFileURL?.path ?? audio.micAudioFileURL?.path
            let notes = meetingNotes
            lastEndedMeetingNotes = (id, notes)
            activeMeetingId = nil
            activeMeetingStartedAt = nil
            activeMeetingTitle = nil
            meetingNotes = ""
            if let store = coreStore {
                CoreMemoryStore.accessQueue.async {
                    MeetingContinuation.completeRecording(store: store, meetingId: id, partIndex: partIndex, continued: continued,
                                                          endUnixMs: endMs, audioPath: audioPath)
                }
            }
            switch MeetingContinuation.partEnding(partIndex: partIndex, continued: continued, hasAudio: audioPath != nil) {
            case .transcribe(let index):
                transcribePart(meetingId: id, partIndex: index, audioPath: audioPath, notes: notes, autoTrim: autoTrim)
            case .nothing:
                break
            case .unknownContinuedPart:
                MeetingContinuation.log.error("meeting \(id): ended a continued recording with no part index; not transcribed (Transcribe again can)")
            }
        }
    }

    private var callWaitTimer: Timer?

    /// Quit mid-recording (spec §6): stop the audio and complete the meeting and its part
    /// with the audio path, waiting (up to 5 s) since the process exits before an async block
    /// runs. No transcription here; the id is marked so the next launch transcribes it.
    private func endMeetingForQuit() {
        // A transcription cut short by the quit is picked up at launch too.
        var marks = Set(transcriptionQueue?.pendingMeetingIds ?? [])
        if let processingMeetingId { marks.insert(processingMeetingId) }
        let active = activeMeetingId
        var audioPath: String?
        if active != nil {
            captureGeneration += 1
            audio.stop()
            meetingEndDetector.stop()
            stopCheckIn()
            meetingSnapshotService?.stop()
            audioPath = audio.systemAudioFileURL?.path ?? audio.micAudioFileURL?.path
            if audioPath != nil, let active { marks.insert(active) }
        }
        // Marked first: if the process dies mid-write, the row stays `.recording` and the
        // launch re-link handles it; launch ignores a mark whose meeting isn't completed.
        if !marks.isEmpty {
            let pending = UserDefaults.standard.array(forKey: Self.pendingLaunchTranscriptionKey) as? [Int64] ?? []
            UserDefaults.standard.set(Array(marks.union(pending)), forKey: Self.pendingLaunchTranscriptionKey)
        }
        guard let id = active else { return }
        let partIndex = activePartIndex
        let continued = activeMeetingIsContinued
        let notes = meetingNotes
        activeMeetingId = nil
        activeMeetingStartedAt = nil
        guard let store = coreStore else { return }
        // Bounded: a long job ahead on accessQueue must not hang the quit. On timeout the
        // row stays `.recording` and the launch re-link recovers it.
        let done = DispatchSemaphore(value: 0)
        CoreMemoryStore.accessQueue.async {
            // Notes live only in memory while recording; the launch transcription reads them back.
            if !notes.isEmpty { store.setMeetingNotes(meetingId: id, notes: notes) }
            MeetingContinuation.completeRecording(store: store, meetingId: id, partIndex: partIndex, continued: continued,
                                                  endUnixMs: Int64(Date().timeIntervalSince1970 * 1000), audioPath: audioPath)
            done.signal()
        }
        if done.wait(timeout: .now() + 5) == .timedOut {
            MeetingContinuation.log.error("meeting \(id): quit before its row was completed (accessQueue busy); launch re-links it")
        }
    }

    /// Away Mode takes an eligible prompt instead of the alert panel and joins at the
    /// meeting's start (the prompt fires `leadMinutes` early). The prompt stays pending
    /// meanwhile. If the user turns out to be present (in the call during the lead window,
    /// or at the start) or Away Mode is off by then, the prompt goes back to the panel.
    private func scheduleAwayRun(for meeting: QualifyingMeeting) {
        Task { @MainActor [weak self] in
            while case let wait = meeting.startDate.timeIntervalSinceNow, wait > 0 {
                try? await Task.sleep(nanoseconds: UInt64(min(5, wait) * 1_000_000_000))
                guard let self else { return }
                if self.awayCoordinator.userIsPresent { break }
            }
            guard let self else { return }
            var result = AwayRunResult.stoodAside
            if !self.awayCoordinator.userIsPresent { result = await self.awayCoordinator.run(for: meeting) }
            guard self.meetingTrigger.pendingPrompt?.id == meeting.id else { return }
            if result == .stoodAside {
                self.promptNormally(meeting)
            } else {
                self.meetingTrigger.dismissPrompt()
            }
        }
    }

    private func showMeetingAlert(for prompt: QualifyingMeeting, autoStart: Bool) {
        meetingAlertPanel.onOpenPrep = { [weak self] key in self?.showPrepBrief(key: key) }
        defer { lookUpAlertPrep(for: prompt) }
        meetingAlertPanel.show(
            for: prompt,
            autoStart: autoStart,
            onAccept: {
                Task { @MainActor in self.meetingTrigger.acceptPrompt() }
            },
            onAcceptAndOpenTimeline: {
                Task { @MainActor in
                    self.meetingTrigger.acceptPrompt()
                    self.openTimeline?()
                }
            },
            onDismiss: {
                Task { @MainActor in self.meetingTrigger.dismissPrompt() }
            }
        )
    }

    /// Offers "Open brief" on the meeting-start pop-up when the meeting has a readable brief.
    private func lookUpAlertPrep(for meeting: QualifyingMeeting) {
        guard let coreStore else { return }
        nonisolated(unsafe) let store = coreStore
        let key = TodayLogic.prepKey(eventId: meeting.id, start: meeting.startDate)
        let meetingId = meeting.id
        CoreMemoryStore.accessQueue.async {
            let found = store.meetingPrep(key: key)?.brief != nil
            DispatchQueue.main.async { [weak self] in
                self?.meetingAlertPanel.setPrep(key: found ? key : nil, forMeetingId: meetingId)
            }
        }
    }

    /// A completed meeting with a part whose recording is still on disk but has no
    /// transcript, e.g. one whose transcription was cut short by the app quitting.
    /// Checked off main (`TranscribeAgainAvailability`); false until that check lands.
    func canTranscribeAgain(_ meeting: CoreMeeting) -> Bool {
        transcribeAgainAvailability?.canTranscribeAgain(meeting) ?? false
    }

    /// The last transcription of this meeting threw; the recording is still there.
    func transcriptionFailed(_ meeting: CoreMeeting) -> Bool {
        transcribeAgainAvailability?.failedIds.contains(meeting.id) ?? false
    }

    /// The last transcription ran and found no words.
    func noSpeechDetected(_ meeting: CoreMeeting) -> Bool {
        transcribeAgainAvailability?.noSpeechIds.contains(meeting.id) ?? false
    }

    /// Views read it through `canTranscribeAgain`; its changes re-render AppState's observers.
    private lazy var transcribeAgainAvailability: TranscribeAgainAvailability? = coreStore.map { store in
        let availability = TranscribeAgainAvailability(store: store)
        availability.objectWillChange.sink { [weak self] _ in self?.objectWillChange.send() }.store(in: &cancellables)
        return availability
    }

    /// Re-transcribes every part with audio on disk; each part's segments are replaced.
    func transcribeAgain(_ meeting: CoreMeeting) {
        guard processingMeetingId == nil, canTranscribeAgain(meeting), let transcriptionQueue,
              mediaRetention.workingMeetingId != meeting.id else { return }  // its audio may be mid-move
        markProcessing(meeting.id)
        transcriptionQueue.transcribeAgain(meetingId: meeting.id, notes: meeting.notes ?? "")
    }

    /// Transcribes every part again in `language` (a whisper code; nil detects) and stores it.
    /// The caller has checked that a part's audio is on disk (`MeetingOptionsMenu`).
    func transcribeAgain(_ meeting: CoreMeeting, language: String?) {
        guard processingMeetingId == nil, meeting.status == .completed, let transcriptionQueue,
              mediaRetention.workingMeetingId != meeting.id else { return }
        markProcessing(meeting.id)
        transcriptionQueue.transcribeAgain(meetingId: meeting.id, notes: meeting.notes ?? "", language: .some(language))
    }

    /// FR-MTG-5: queues one part's transcription (`MeetingTranscriptionQueue`, one part at
    /// a time, in order); the last part to land summarizes the whole meeting.
    private func transcribePart(meetingId: Int64, partIndex: Int64, audioPath: String?, notes: String, autoTrim: Bool = false) {
        guard let transcriptionQueue else {
            // No queue means no store, so there is nothing to recover this launch. Marking the
            // meeting lets the next launch's reconcile pick it up (it checks for audio and status).
            Logger(subsystem: "com.mpointer.kestrel", category: "transcription-queue")
                .error("meeting \(meetingId) part \(partIndex): no transcription queue; marked for the next launch")
            let pending = UserDefaults.standard.array(forKey: Self.pendingLaunchTranscriptionKey) as? [Int64] ?? []
            UserDefaults.standard.set(Array(Set(pending).union([meetingId])), forKey: Self.pendingLaunchTranscriptionKey)
            return
        }
        if audioPath != nil { markProcessing(meetingId) }
        transcriptionQueue.enqueue(.init(meetingId: meetingId, partIndex: partIndex, audioPath: audioPath, notes: notes, autoTrim: autoTrim))
    }

    /// "Keep media" switch: exempts this device's own copy of the meeting from the
    /// retention media steps. `onFailure` runs on main when nothing was written.
    func setKeepMedia(_ meeting: CoreMeeting, keep: Bool, onFailure: @escaping () -> Void = {}) {
        guard let store = coreStore, let id = meeting.ownCopyId(deviceId: deviceId) else { return onFailure() }
        CoreMemoryStore.accessQueue.async {
            let ok = store.setKeepMedia(meetingId: id, keep: keep)
            DispatchQueue.main.async {
                if ok { MeetingLifecycle.postMeetingsChanged([id, meeting.id]) } else { onFailure() }
            }
        }
    }

    /// The new meeting from a split waits here until the original's summary finishes.
    private var pendingSplitSummary: Int64?

    /// "Split into separate meetings": the last part becomes its own meeting; both are
    /// summarized again, one after the other.
    func splitLastPart(of meeting: CoreMeeting, onSnapshotMoveFailed: @escaping () -> Void = {}) {
        guard processingMeetingId == nil, let store = coreStore, let transcriptionQueue else { return }
        markProcessing(meeting.id)
        processingStage = "Splitting…"
        let storage = makeSnapshotStorage()
        MeetingLifecycle.splitLastPart(store: store, meetingId: meeting.id, makeFolder: { try storage.makeMeetingFolder(start: $0, title: $1) }) { [weak self] outcome in
            guard let self else { return }
            guard let outcome else { self.clearProcessingState(for: meeting.id); return }
            if outcome.snapshotMoveFailed { onSnapshotMoveFailed() }
            self.pendingSplitSummary = outcome.newMeetingId
            transcriptionQueue.summarizeExisting(meetingId: meeting.id, notes: meeting.notes ?? "")
        }
    }

    /// "Merge with previous meeting": `ownId` (this device's copy) joins `previousId` as its
    /// next parts, then the merged meeting is transcribed and summarized again.
    func mergeWithPrevious(ownId: Int64, into previousId: Int64) {
        guard processingMeetingId == nil, let store = coreStore, let transcriptionQueue,
              ![ownId, previousId].contains(mediaRetention.workingMeetingId) else { return }
        markProcessing(previousId)
        processingStage = "Merging…"
        let storage = makeSnapshotStorage()
        MediaMover.queue.async {
            let notes: String? = CoreMemoryStore.accessQueue.sync {
                guard MeetingSplit.merge(store: store, into: previousId, from: ownId,
                                         makeFolder: { try storage.makeMeetingFolder(start: $0, title: $1) }) else { return nil }
                return store.getMeeting(id: previousId)?.notes ?? ""
            }
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                MeetingLifecycle.postMeetingsChanged([previousId, ownId])
                guard let notes else {
                    MeetingSplit.log.error("meeting \(ownId): merge into \(previousId) failed; left as is")
                    self.clearProcessingState(for: previousId)
                    return
                }
                transcriptionQueue.transcribeAgain(meetingId: previousId, notes: notes)
            }
        }
    }

    /// "Trim recording": keeps the first `keepMs` (transcript clock) of a meeting this Mac
    /// recorded. Core trims the rows and syncs the trim; then the spanning part's audio is cut,
    /// the removed parts' and snapshots' files go, and the summary is regenerated as the
    /// Regenerate action does it, with the routing defaults.
    /// `onFailure` runs on main when the store couldn't trim, `onNothingToTrim` when it had
    /// nothing to cut (`keepMs` not inside the meeting, or the meeting changed meanwhile).
    func trimMeeting(_ meeting: CoreMeeting, keepMs: Int64, onFailure: @escaping () -> Void = {},
                     onNothingToTrim: @escaping () -> Void = {}) {
        let id = meeting.id
        guard processingMeetingId == nil, meeting.deviceId == deviceId, let store = coreStore,
              mediaRetention.workingMeetingId != id else { return }
        markProcessing(id)
        processingStage = "Trimming…"
        CoreMemoryStore.accessQueue.async { [weak self] in
            let trim: AudioTrimmer.CoreTrim?
            do { trim = try AudioTrimmer.trimStored(store: store, meetingId: id, keepMs: keepMs) } catch {
                DispatchQueue.main.async { onFailure(); self?.clearProcessingState(for: id) }
                return
            }
            let hasTranscript = trim != nil
                && !(store.meetingTranscriptText(meetingId: id) ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            Task { @MainActor in
                if let trim { await trim.applyToFiles() } else { onNothingToTrim() }
                guard let self else { return }
                MeetingLifecycle.postMeetingsChanged([id])
                guard hasTranscript else { return self.clearProcessingState(for: id) }
                self.processingStage = "Summarizing…"
                self.regenerateMeetingSummary(
                    meetingId: id, notes: meeting.notes ?? "", transcript: "",
                    ollamaModel: self.routing.ollamaDefaultModel,
                    routing: self.routing.routingConfig(preferredProvider: self.routing.preferredProvider,
                                                        preferredModel: self.routing.preferredModel),
                    customInstructions: self.promptLibrary.instructions[.meetingSummary] ?? ""
                ) { [weak self] _ in self?.clearProcessingState(for: id) }
            }
        }
    }

    private func markProcessing(_ meetingId: Int64) {
        guard processingMeetingId == nil else { return }
        processingMeetingId = meetingId
        processingStage = "Transcribing…"
    }

    /// The most summaries one import starts, newest first, so a long history can't run up a bill.
    static let importSummaryLimit = 25

    /// Imports off the main thread, then summarizes the newest new meetings.
    func runImport(_ work: @escaping (CoreMemoryStore) -> CoreImportOutcome?, done: @escaping (CoreImportOutcome?) -> Void) {
        guard let coreStore else { return done(nil) }
        CoreMemoryStore.accessQueue.async { [weak self] in
            let outcome = work(coreStore)
            // The job writes its notes back, so it carries what the import stored.
            let jobs = (outcome?.meetingIds.prefix(Self.importSummaryLimit) ?? []).map {
                MeetingTranscriptionQueue.Job(meetingId: $0, partIndex: 1, audioPath: nil, notes: coreStore.getMeeting(id: $0)?.notes ?? "")
            }
            DispatchQueue.main.async {
                jobs.forEach { self?.transcriptionQueue?.enqueue($0) }
                done(outcome)
            }
        }
    }

    /// Summaries run on the slow lane with the routing defaults read on main at that moment.
    private lazy var transcriptionQueue: MeetingTranscriptionQueue? = coreStore.map { store in
        MeetingTranscriptionQueue(
            store: store,
            summarize: { [weak self] meetingId, notes, transcript in
                DispatchQueue.main.async { [weak self] in
                    guard let self else { return }
                    self.transcribeAgainAvailability?.refresh(meetingId)
                    self.processingMeetingId = meetingId
                    self.processingStage = "Summarizing…"
                    let ollamaModel = self.routing.ollamaDefaultModel
                    let routingConfig = self.routing.routingConfig(preferredProvider: self.routing.preferredProvider, preferredModel: self.routing.preferredModel)
                    let instructions = self.promptLibrary.instructions[.meetingSummary] ?? ""
                    let slow = self.slowStore
                    CoreMemoryStore.slowLaneQueue.async { [weak self] in
                        let slides = slow?.meetingSlidesText(meetingId: meetingId) ?? ""
                        let outcome = slow?.summarizeMeeting(meetingId: meetingId, notes: notes, transcript: transcript, slides: slides, model: ollamaModel, routing: routingConfig.withTypesafeKey(self?.freshTitleCheckKey()), customInstructions: instructions)
                        DispatchQueue.main.async {
                            self?.recordSummaryOutcome(outcome, meetingId: meetingId)
                            self?.clearProcessingState(for: meetingId)
                        }
                    }
                }
            },
            // The summary's model and routing. A failure names nobody and the summary still runs.
            nameSpeakers: { [weak self] meetingId, voices in
                typealias Naming = (store: CoreMemoryStore, model: String, routing: CoreRoutingConfig, remember: Bool)
                let naming = DispatchQueue.main.sync { () -> Naming? in
                    guard let self, let slow = self.slowStore else { return nil }
                    let routing = self.routing.routingConfig(preferredProvider: self.routing.preferredProvider, preferredModel: self.routing.preferredModel)
                    return (slow, self.routing.ollamaDefaultModel, routing, self.speakerNamesSettings.rememberVoices)
                }
                guard let naming else { return }
                let named = CoreMemoryStore.slowLaneQueue.sync {
                    naming.store.assignMeetingSpeakers(meetingId: meetingId, voices: voices, model: naming.model, routing: naming.routing, remember: naming.remember)
                }
                if named == nil {
                    Logger(subsystem: "com.mpointer.kestrel", category: "transcription-queue").error("meeting \(meetingId): speakers not named")
                }
            },
            // "Still in a meeting?": the rows, then the files, before naming and the summary.
            trim: { meetingId, keepMs in
                let trim = CoreMemoryStore.accessQueue.sync { try? AudioTrimmer.trimStored(store: store, meetingId: meetingId, keepMs: keepMs) }
                guard let trim = trim ?? nil else { return }
                let done = DispatchSemaphore(value: 0)
                Task.detached { await trim.applyToFiles(); done.signal() }
                done.wait()
                DispatchQueue.main.async { MeetingLifecycle.postMeetingsChanged([meetingId]) }
            },
            onStage: { [weak self] meetingId, stage in
                guard let self else { return }
                if let stage {
                    self.processingMeetingId = meetingId
                    self.processingStage = stage
                } else {
                    self.clearProcessingState(for: meetingId)
                    self.transcribeAgainAvailability?.refresh(meetingId)
                }
            },
            onFailure: { [weak self] meetingId in self?.transcribeAgainAvailability?.markFailed(meetingId) },
            modelsDownloading: { [weak self] in
                DispatchQueue.main.sync {
                    if case .downloading = self?.modelBootstrap.phase { return true }
                    return false
                }
            }
        )
    }

    /// Turns Reply Priorities on or off. Turning it on queues scoring for the threads
    /// already waiting on the user.
    func setReplyPriorityEnabled(_ on: Bool) {
        replyPriority.enabled = on
        guard on, let store = coreStore else { return }
        CoreMemoryStore.accessQueue.async { _ = store.seedThreadPriorityJobs() }
    }

    /// Turns topic judging on or off. Turning it on shows the estimate for the user to
    /// confirm; turning it off stops judging and deletes nothing.
    func setTopicSortEnabled(_ on: Bool) {
        if on, topicSort.overBudget { topicSort.requireStart() }
        topicSort.enabled = on
        refreshTopicSortEstimate()
    }

    /// Fetches the judging estimate while the switch is on and Start was not pressed. Also
    /// the "Try again" button, and re-run by `scheduleTopicsStep` until sorting is done.
    func refreshTopicSortEstimate() {
        guard topicSort.awaitingStart, let store = coreStore else { return }
        CoreMemoryStore.accessQueue.async { [weak self] in
            let e = store.topicJudgeEstimate(usdPerMillion: TopicSortSettings.usdPerMillion)
            DispatchQueue.main.async { self?.topicSort.setEstimate(e) }
        }
    }

    /// The user pressed Start on the estimate: allow judging up to 1.5x the estimated tokens.
    func startTopicSort() {
        guard let store = coreStore, let e = topicSort.estimate else { return }
        let budget = Int64((Double(e.tokens) * 1.5).rounded(.up))
        CoreMemoryStore.accessQueue.async { [weak self] in
            let ok = store.topicJudgeStart(budgetTokens: budget)
            DispatchQueue.main.async {
                if ok { self?.topicSort.markStarted() } else { self?.topicSort.setStartFailed(true) }
            }
        }
    }

    /// Turns "Find steps with TypeSafe" on or off. On shows the estimate to confirm; off
    /// sends the next batches to the local model.
    func setTopicStepsEnabled(_ on: Bool) {
        if on, topicSteps.overBudget { topicSteps.requireStart() }
        topicSteps.enabled = on
        refreshTopicStepsEstimate()
    }

    /// Fetches the steps estimate while the switch is on and Start was not pressed.
    func refreshTopicStepsEstimate() {
        guard topicSteps.awaitingStart, let store = coreStore else { return }
        CoreMemoryStore.accessQueue.async { [weak self] in
            let e = store.topicStepsEstimate(usdPerMillion: TopicSortSettings.usdPerMillion)
            DispatchQueue.main.async { self?.topicSteps.setEstimate(e) }
        }
    }

    /// The user pressed Start on the steps estimate: allow up to 1.5x the estimated tokens.
    func startTopicSteps() {
        guard let store = coreStore, let e = topicSteps.estimate else { return }
        let budget = Int64((Double(e.tokens) * 1.5).rounded(.up))
        CoreMemoryStore.accessQueue.async { [weak self] in
            let ok = store.topicStepsStart(budgetTokens: budget)
            DispatchQueue.main.async {
                if ok { self?.topicSteps.markStarted() } else { self?.topicSteps.setStartFailed(true) }
            }
        }
    }

    // MARK: - Live question alerts

    private func feedLiveQuestionWatcher(_ rows: [CoreLiveSegment]) {
        guard let id = activeMeetingId, let first = rows.first, first.meetingId == id else { return }
        if liveQuestionWatcher?.meetingId != id {
            let names = profile.liveQuestionNames
            guard LiveQuestionWatcher.shouldRun(settings: liveQuestionSettings, names: names) else { return }
            liveQuestionWatcher = makeLiveQuestionWatcher(meetingId: id, names: names)
        }
        liveQuestionWatcher?.ingest(rows)
    }

    /// Detection and drafting use Ask's model and routing (local-only mode, spend cap) and run on
    /// the slow lane; the alert rows go through `accessQueue`.
    private func makeLiveQuestionWatcher(meetingId: Int64, names: [String]) -> LiveQuestionWatcher {
        let log = Logger(subsystem: "com.mpointer.kestrel", category: "live-question")
        let store = coreStore
        return LiveQuestionWatcher(
            meetingId: meetingId, meetingTitle: activeMeetingTitle ?? String(localized: "Meeting"), names: names,
            detect: { [weak self] lines, index in
                guard let (slow, model, routing) = self?.liveQuestionModel() else { return nil }
                return await AppState.run(on: CoreMemoryStore.slowLaneQueue) {
                    switch slow.detectLiveQuestion(lines: lines, index: index, names: names, model: model, routing: routing) {
                    case .success(let verdict): return verdict
                    case .failure(let error): log.error("live question detect failed: \(String(describing: error), privacy: .public)")
                    case nil: log.error("live question detect: invalid arguments")
                    }
                    return nil
                }
            },
            draft: { [weak self] verdict, recent in
                guard let (slow, model, routing) = self?.liveQuestionModel() else { return nil }
                return await AppState.run(on: CoreMemoryStore.slowLaneQueue) {
                    switch slow.draftLiveAnswer(verdict: verdict, recent: recent, model: model, routing: routing) {
                    case .success(let answer): return answer
                    case .failure(let error): log.error("live question draft failed: \(String(describing: error), privacy: .public)")
                    case nil: log.error("live question draft: invalid arguments")
                    }
                    return nil
                }
            },
            deliver: liveAlertDelivery, now: Date.init,
            insert: { alert in
                guard let store else { return nil }
                return await AppState.run(on: CoreMemoryStore.accessQueue) { store.insertLiveAlert(alert) }
            },
            saveDraft: { id, answer in
                guard let store else { return }
                _ = await AppState.run(on: CoreMemoryStore.accessQueue) { store.setLiveAlertDraft(id: id, draft: answer) }
            },
            seed: {
                guard let store else { return ([], 0) }
                return await AppState.run(on: CoreMemoryStore.accessQueue) {
                    (store.listLiveAlerts(meetingId: meetingId), LiveQuestionWatcher.partOffsetCs(store.meetingParts(meetingId: meetingId) ?? []))
                }
            }
        )
    }

    /// The email goes only to the default connected Gmail account (`GmailSelfSend`); status,
    /// evidence and context lookups go through `accessQueue`.
    private func makeLiveAlertDelivery() -> LiveAlertDelivery {
        let store = coreStore
        let gmail = gmailConnectorState
        let settings = liveQuestionSettings
        let deviceId = deviceId
        let log = Logger(subsystem: "com.mpointer.kestrel", category: "live-question")
        let selfSend = GmailSelfSend(
            connectedAccount: { [weak gmail] in gmail?.defaultEmail },
            token: { [weak gmail] email in
                await RoutineScheduler.fetchAccessToken(from: gmail?.oauthHelper(forEmail: email), requiredScope: RoutineScheduler.gmailWriteScope, log: log)
            },
            transport: { plan, token in await RoutineScheduler.performToolCall(plan: plan, accessToken: token) }
        )
        return LiveAlertDelivery(
            deviceId: deviceId,
            alertsEnabled: { settings.enabled },
            post: LiveAlertDelivery.postToNotificationCenter,
            sendEmail: { subject, body in await selfSend.send(subject: subject, body: body) },
            setEmailStatus: { id, status in
                guard let store else { return }
                _ = await AppState.run(on: CoreMemoryStore.accessQueue) { store.setLiveAlertEmailStatus(id: id, status: status) }
            },
            emailEnabled: { settings.emailEnabled },
            userAway: { [weak self] in
                LiveAlertDelivery.isAway(
                    awayModeRunning: self?.awayCoordinator.isRunning ?? false,
                    idleSeconds: CGEventSource.secondsSinceLastEventType(.hidSystemState, eventType: .null)
                )
            },
            evidence: { refs in
                guard let store, !refs.isEmpty else { return [] }
                return await AppState.run(on: CoreMemoryStore.accessQueue) { LiveAlertDelivery.evidenceLines(refs, store: store, origin: deviceId) }
            },
            context: { alert in
                guard let store, let meetingId = alert.meetingId else { return [] }
                return await AppState.run(on: CoreMemoryStore.accessQueue) {
                    let offset = LiveQuestionWatcher.partOffsetCs(store.meetingParts(meetingId: meetingId) ?? [])
                    return LiveAlertDelivery.contextLines(store.listLiveSegments(meetingId: meetingId), atCs: alert.atCs, partOffsetCs: offset)
                }
            }
        )
    }

    /// The slow store, Ask's Ollama model and a fresh routing config (keys read per call).
    private func liveQuestionModel() -> (CoreMemoryStore, String, CoreRoutingConfig)? {
        guard let slow = slowStore else { return nil }
        return (slow, routing.ollamaDefaultModel, routing.routingConfig(preferredProvider: routing.preferredProvider, preferredModel: routing.preferredModel))
    }

    private nonisolated static func run<T>(on queue: DispatchQueue, _ body: @escaping () -> T) async -> T {
        await withCheckedContinuation { continuation in queue.async { continuation.resume(returning: body()) } }
    }

    /// Drives the shared jobs-queue mechanism (`Store::run_jobs_batch`: `reindex_sweep`
    /// and `thread_state_sync`) forward, one job per turn on
    /// `CoreMemoryStore.accessQueue`, gated on `PowerIdleGate` before each call into
    /// Rust -- Rust itself stays completely unaware of power/idle state
    /// (docs/superpowers/specs/2026-08-07-job-queue-design.md). Replaces the previous
    /// `scheduleEmbeddingReindexStep`, which called `reindexBatch` directly with no
    /// gating -- the `reindex_sweep` job type wraps that same `reindex_batch` call
    /// unchanged, so real backfill behavior (throughput, per-row timing) is identical;
    /// only the scheduling/gating wrapper around it changed. Also drives
    /// `thread_summarize` forward each turn via `runThreadSummarizeBatch` -- it needs a
    /// live Ollama call, so unlike `reindex_sweep`/`thread_state_sync` it isn't
    /// dispatched through `run_jobs_batch` itself
    /// (docs/superpowers/specs/2026-08-12-l2-episodes-design.md).
    ///
    /// `episode_segment`/`episode_summarize` are deliberately NOT driven here --
    /// see `scheduleEpisodeSegmentStep()` (#52): gating episode processing behind the
    /// same AC+idle check as `reindex_sweep`'s heavier embedding-compute work left
    /// Episodes barely functional for anyone working unplugged or rarely idle.
    ///
    /// Two distinct reschedule paths: if the power/idle gate isn't satisfied, recheck
    /// after 30s (conditions like "user went idle" or "got plugged in" change slowly,
    /// no need to poll every 100ms). If the gate passes and there's more work
    /// (`processed > 0`), continue at the same 0.1s cadence the pre-migration loop
    /// used. If nothing was eligible to run, drop to that same slower 30s cadence
    /// rather than stopping -- Whimbrel is a long-running menubar app and this work must
    /// keep picking up indefinitely, so going dormant after the first catch-up would
    /// stall it until the next relaunch.
    private func scheduleJobsBatchStep() {
        guard let store = coreStore else { return }
        // Read on the main thread and capture -- `routing`/`promptLibrary` (@Published)
        // aren't safe to read from the background queue below, same precedent as
        // `transcriptionQueue`'s summary, which reads `routing.ollamaDefaultModel` on main.
        let ollamaModel = routing.ollamaDefaultModel
        let threadSummaryInstructions = promptLibrary.instructions[.threadSummary] ?? ""
        let slow = slowStore
        CoreMemoryStore.accessQueue.asyncAfter(deadline: .now() + 0.1) { [weak self] in
            guard PowerIdleGate.isEligibleForBackgroundWork() else {
                DispatchQueue.main.asyncAfter(deadline: .now() + 30) { [weak self] in
                    self?.scheduleJobsBatchStep()
                }
                return
            }
            let processed = store.runJobsBatch(maxJobs: 1)
            CoreMemoryStore.slowLaneQueue.async { [weak self] in
                // "Use local model" off keeps Ollama idle; read on main right before the call
                // (the slow lane never waits on main, so this cannot deadlock).
                let localAllowed: Bool = DispatchQueue.main.sync { [weak self] in
                    self.map { RoutingSettings.localSummariesAllowed(useLocalModel: $0.routing.useLocalModel, localOnlyMode: $0.routing.localOnlyMode) } ?? false
                }
                let threadsSummarized = localAllowed
                    ? (slow?.runThreadSummarizeBatch(model: ollamaModel, maxJobs: 1, customInstructions: threadSummaryInstructions) ?? 0)
                    : 0
                guard processed > 0 || threadsSummarized > 0 else {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 30) { [weak self] in
                        self?.scheduleJobsBatchStep()
                    }
                    return
                }
                DispatchQueue.main.async { [weak self] in
                    self?.scheduleJobsBatchStep()
                }
            }
        }
    }

    /// Drives `episode_segment` (`Store::run_episode_segment_batch`) and
    /// `episode_summarize` (`runEpisodeSummarizeBatch`) forward every turn, same
    /// throttled 0.1s-when-busy/30s-when-idle cadence as `scheduleJobsBatchStep()`.
    /// Segmenting runs WITHOUT the `PowerIdleGate` check (#52); the summary waits for it. Episodes are meant to be a
    /// reliable, always-on record of the day's activity; gating them behind AC power +
    /// 60s of continuous idle meant they only ever processed during idle-while-plugged-in
    /// moments -- rare for a laptop used unplugged, and rare during genuinely focused
    /// work -- so real activity went unsegmented for hours while the few windows that DID
    /// open were disproportionately populated by low-value content (e.g. the lock screen,
    /// itself an idle-triggering event). Segmenting is cheap SQL grouping on
    /// `accessQueue`; the summary is a local Ollama call that can take minutes, so it runs
    /// on the slow lane, where it can't hold up UI reads.
    private func scheduleEpisodeSegmentStep() {
        guard let store = coreStore else { return }
        let ollamaModel = routing.ollamaDefaultModel
        let episodeSummaryInstructions = promptLibrary.instructions[.episodeSummary] ?? ""
        let slow = slowStore
        CoreMemoryStore.accessQueue.asyncAfter(deadline: .now() + 0.1) { [weak self] in
            let segmented = store.runEpisodeSegmentBatch(maxJobs: 1)
            CoreMemoryStore.slowLaneQueue.async { [weak self] in
                // Segmenting stays always-on (#52); the Ollama summary waits for the same
                // AC-power + idle gate as thread summaries, as on Windows, so a backlog
                // doesn't load the summary model while the user is working.
                // Also skipped while "Use local model" is off; read on main right before the call
                // (the slow lane never waits on main, so this cannot deadlock).
                let localAllowed: Bool = DispatchQueue.main.sync { [weak self] in
                    self.map { RoutingSettings.localSummariesAllowed(useLocalModel: $0.routing.useLocalModel, localOnlyMode: $0.routing.localOnlyMode) } ?? false
                }
                let summarized = PowerIdleGate.isEligibleForBackgroundWork() && localAllowed
                    ? (slow?.runEpisodeSummarizeBatch(model: ollamaModel, maxJobs: 1, customInstructions: episodeSummaryInstructions) ?? 0)
                    : 0
                guard segmented > 0 || summarized > 0 else {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 30) { [weak self] in
                        self?.scheduleEpisodeSegmentStep()
                    }
                    return
                }
                DispatchQueue.main.async { [weak self] in
                    self?.scheduleEpisodeSegmentStep()
                }
            }
        }
    }

    /// Scores `waiting_on_me` threads with TypeSafe (`runThreadPriorityBatch`). Ungated by
    /// `PowerIdleGate`: it is a ~1s HTTPS request, not an Ollama job, so new mail must be
    /// ranked within minutes even on battery or while the user works. The opt-in/Local-only
    /// gate is re-read on the main queue right before each call, so a toggle-off takes
    /// effect on the very next call. Nothing on main waits on the slow lane (never call
    /// `slowLaneQueue.sync` from main), so this `main.sync` cannot deadlock.
    private func scheduleReplyPriorityStep() {
        guard let slow = slowStore else { return }
        CoreMemoryStore.slowLaneQueue.asyncAfter(deadline: .now() + 0.1) { [weak self] in
            let key: String? = DispatchQueue.main.sync { [weak self] in
                self?.replyPriority.activeKey(localOnly: self?.routing.localOnlyMode ?? true)
            }
            var prioritized: Int64 = 0
            if let key {
                let result = slow.runThreadPriorityBatch(apiKey: key, maxJobs: 1)
                DispatchQueue.main.async { [weak self] in
                    if result == -2 { self?.replyPriority.markKeyRejected(true) }
                    else if result > 0 { self?.replyPriority.markKeyRejected(false) }
                }
                prioritized = max(result, 0)
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + (prioritized > 0 ? 0 : 30)) { [weak self] in
                self?.scheduleReplyPriorityStep()
            }
        }
    }

    /// Topics: the alias/index batch always runs (free, local); TypeSafe judging runs after
    /// it only when the switch is on and Start was pressed. Same slow lane and main.sync
    /// key read as `scheduleReplyPriorityStep`.
    private func scheduleTopicsStep() {
        guard let slow = slowStore else { return }
        CoreMemoryStore.slowLaneQueue.asyncAfter(deadline: .now() + 0.1) { [weak self] in
            let more = slow.runTopicsBatch()
            let key: String? = DispatchQueue.main.sync { [weak self] in
                self?.topicSort.activeKey(localOnly: self?.routing.localOnlyMode ?? true)
            }
            var judged: Int64 = 0
            if let key {
                let result = slow.runTopicJudgeBatch(apiKey: key, max: 8, usdPerMillion: TopicSortSettings.usdPerMillion)
                judged = result
                DispatchQueue.main.async { [weak self] in
                    if result == -2 { self?.topicSort.markKeyRejected(true) }
                    else if result == -3 { self?.topicSort.setOverBudget(true) }
                    else if result == -5 {
                        // Core has no budget (database replaced): show the estimate and Start.
                        self?.topicSort.requireStart()
                        self?.refreshTopicSortEstimate()
                    }
                    // -4: today's allowance is used up; retried on the idle cadence below.
                    else if result > 0 { self?.topicSort.markKeyRejected(false) }
                }
            } else if more == 0 {
                // Waiting on Start: once the batch is idle, refresh an estimate that was
                // still sorting (or empty).
                DispatchQueue.main.async { [weak self] in
                    guard let self, self.topicSort.awaitingStart,
                          self.topicSort.estimate.map({ $0.indexing || $0.sources == 0 }) ?? false else { return }
                    self.refreshTopicSortEstimate()
                }
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + (more == 1 || judged > 0 ? 0.1 : 60)) { [weak self] in
                self?.scheduleTopicsStep()
            }
        }
    }

    /// Publishes topic pages for the phone, once this launch has finished a content-sync
    /// round, then refreshes the Obsidian vault when it is switched on and a folder is chosen.
    /// Every 300 s, first run about 60 s after launch; a failure is stored for Settings and
    /// waits for the next tick. The vault settings live in UserDefaults (see `VaultSettings`),
    /// so the slow lane reads them directly.
    private func scheduleTopicVaultStep(after delay: Double = 300) {
        guard let slow = slowStore else { return }
        CoreMemoryStore.slowLaneQueue.asyncAfter(deadline: .now() + delay) { [weak self] in
            // A computer that was off publishes only after it has pulled what the others did
            // meanwhile: its build is the newest, so a stale page would win on every device.
            // `lastSyncedAt` is set by this launch's first good round. The vault is local only.
            let pulled = DispatchQueue.main.sync { self?.contentSyncCoordinator.lastSyncedAt != nil }
            // 0 is expected until the store has its own device id.
            if pulled, slow.publishTopicSnapshots(builtBy: Host.current().localizedName ?? "Mac") < 0 {
                Logger(subsystem: "com.mpointer.kestrel", category: "topics")
                    .error("Publishing topic pages for the phone failed")
            }
            let defaults = UserDefaults.standard
            let path = defaults.string(forKey: VaultSettings.pathKey) ?? ""
            if defaults.bool(forKey: VaultSettings.enabledKey), !path.isEmpty {
                switch slow.updateVault(root: URL(fileURLWithPath: path)) {
                case .success:
                    defaults.set(Date().timeIntervalSince1970, forKey: VaultSettings.lastUpdatedKey)
                    defaults.set(false, forKey: VaultSettings.failedKey)
                case .failure(let error):
                    Logger(subsystem: "com.mpointer.kestrel", category: "vault")
                        .error("Vault update failed: \(error.message, privacy: .private)")
                    defaults.set(true, forKey: VaultSettings.failedKey)
                }
            }
            DispatchQueue.main.async { self?.scheduleTopicVaultStep() }
        }
    }

    /// Finds steps in topic sources, under the AC + idle gate. Switch off: the local model
    /// (never a hosted one). Switch on: TypeSafe, only once Start was pressed, a key is saved
    /// and Local-only mode is off; until then nothing runs, so the queue the estimate covers
    /// is left for TypeSafe. Inputs are read in one `main.sync`, as in `scheduleTopicsStep`.
    private func scheduleTopicStepsStep(after delay: Double = 0.1) {
        guard let slow = slowStore else { return }
        CoreMemoryStore.slowLaneQueue.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard PowerIdleGate.isEligibleForBackgroundWork() else {
                DispatchQueue.main.async { self?.scheduleTopicStepsStep(after: 60) }
                return
            }
            typealias Inputs = (routing: CoreRoutingConfig, model: String, key: String?, wait: Bool)
            let inputs = DispatchQueue.main.sync { () -> Inputs? in
                guard let self else { return nil }
                let key = self.topicSteps.enabled ? self.topicSteps.activeKey(localOnly: self.routing.localOnlyMode) : nil
                return (
                    self.routing.routingConfig(preferredProvider: self.routing.preferredProvider, preferredModel: self.routing.preferredModel),
                    self.routing.ollamaDefaultModel,
                    key,
                    // Switch on without a usable key (no Start, no key, Local-only): wait, never local.
                    self.topicSteps.enabled && key == nil
                )
            }
            guard let inputs else { return }
            let result: Int64 = inputs.wait ? 0 : slow.runTopicStepsBatch(
                routing: inputs.routing, ollamaModel: inputs.model, apiKey: inputs.key,
                max: 4, usdPerMillion: TopicSortSettings.usdPerMillion
            )
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                switch result {
                case -2: self.topicSteps.markKeyRejected(true)
                case -3: self.topicSteps.setOverBudget(true)
                case -5:
                    // Core has no budget (database replaced): show the estimate and Start.
                    self.topicSteps.requireStart()
                    self.refreshTopicStepsEstimate()
                case -6: self.topicStepsNoModel = true
                // -4: today's allowance is used up; -1: error. Both retry on the idle cadence.
                case 0...:
                    if inputs.key != nil { self.topicSteps.markKeyRejected(false) } else if !inputs.wait { self.topicStepsNoModel = false }
                default: break
                }
                if result <= 0, self.topicSteps.awaitingStart,
                   self.topicSteps.estimate.map({ $0.indexing || $0.sources == 0 }) ?? false {
                    self.refreshTopicStepsEstimate()
                }
                self.scheduleTopicStepsStep(after: result > 0 ? 0.1 : 60)
            }
        }
    }

    /// Names suggested topics with the user's AI model (10 per run), under the AC + idle gate,
    /// through the routing and spend limits of other background model tasks. Core tries a
    /// placeholder-named topic at most once a day after a failure, so an idle poll is cheap.
    /// Inputs are read in one `main.sync`, as in `scheduleTopicStepsStep`.
    private func scheduleTopicNamingStep(after delay: Double = 120) {
        guard let slow = slowStore else { return }
        CoreMemoryStore.slowLaneQueue.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard PowerIdleGate.isEligibleForBackgroundWork() else {
                DispatchQueue.main.async { self?.scheduleTopicNamingStep(after: 60) }
                return
            }
            typealias Inputs = (routing: CoreRoutingConfig, model: String)
            let inputs = DispatchQueue.main.sync { () -> Inputs? in
                guard let self else { return nil }
                return (
                    self.routing.routingConfig(preferredProvider: self.routing.preferredProvider, preferredModel: self.routing.preferredModel),
                    self.routing.ollamaDefaultModel
                )
            }
            guard let inputs else { return }
            // Only names a model wrote earn the fast cadence: placeholders from a failing model do not.
            let named = slow.runTopicNamingBatch(routing: inputs.routing, ollamaModel: inputs.model, limit: 10)?.modelNamed ?? 0
            DispatchQueue.main.async { [weak self] in
                self?.scheduleTopicNamingStep(after: named > 0 ? 5 : 300)
            }
        }
    }

    /// Drives `CoreSelfTest.ingestNewScreenTextLines` forward on its own periodic
    /// cadence -- same throttled shape as `scheduleEpisodeSegmentStep()`. Before this,
    /// screen-text ingestion (the ONLY path real ambient capture reaches the database --
    /// `ScreenTextCaptureService` only appends to a JSONL log, it doesn't ingest) ran
    /// exclusively from two manual/incidental triggers: the Settings "Run Core Self-Test"
    /// button, or `TimelineView`'s once-per-launch `.onAppear` -- meaning a user whose
    /// daily workflow never opens the Library/Timeline tab got NO new capture data in the
    /// database at all, silently, indefinitely (confirmed: three weeks of a real user's
    /// capture_item rows with zero ingested past the last manual/Timeline trigger).
    private func scheduleScreenTextIngestStep() {
        guard let store = coreStore else { return }
        let deviceId = deviceId
        CoreMemoryStore.accessQueue.asyncAfter(deadline: .now() + 0.1) { [weak self] in
            let outcome = CoreSelfTest.ingestNewScreenTextLines(into: store, deviceId: deviceId)
            guard outcome.hasMoreWork else {
                DispatchQueue.main.asyncAfter(deadline: .now() + 30) { [weak self] in
                    self?.scheduleScreenTextIngestStep()
                }
                return
            }
            DispatchQueue.main.async { [weak self] in
                self?.scheduleScreenTextIngestStep()
            }
        }
    }

    /// Drives LB0-6's transcript-segment backfill to completion in the background, a
    /// few meetings per turn on `CoreMemoryStore.accessQueue` with a short pause
    /// between turns -- same throttled-loop shape as `scheduleJobsBatchStep()`,
    /// so real capture-event dispatch queued on the same serial queue still gets its
    /// normal turn between steps. Stops rescheduling once the batch returns 0 (caught
    /// up) or a negative value (shouldn't happen outside a null handle, but treated
    /// the same as "stop" rather than looping on an error).
    private func scheduleTranscriptBackfillStep() {
        guard let store = coreStore else { return }
        CoreMemoryStore.accessQueue.asyncAfter(deadline: .now() + 0.1) { [weak self] in
            let processed = store.backfillTranscriptSegments(maxMeetings: 5)
            guard processed > 0 else { return }
            DispatchQueue.main.async { [weak self] in
                self?.scheduleTranscriptBackfillStep()
            }
        }
    }

    /// Only clears the shared processing indicator if it still refers to THIS meeting —
    /// guards against a later job (a different, later
    /// meeting) clobbering its own in-flight state if this one somehow finishes after it.
    private func clearProcessingState(for meetingId: Int64) {
        guard processingMeetingId == meetingId else { return }
        processingMeetingId = nil
        processingStage = nil
        MeetingLifecycle.postMeetingsChanged([meetingId])
        if let next = pendingSplitSummary {
            pendingSplitSummary = nil
            markProcessing(next)
            processingStage = "Summarizing…"
            transcriptionQueue?.summarizeExisting(meetingId: next, notes: "")
        }
        // The post-meeting trigger: moves the finished recording to the media folder.
        mediaRetention.run(mode: .daily)
    }

    /// FR-MTG-5 follow-on: regenerates a meeting's summary against its full transcript
    /// using an explicit, one-off provider/model choice (`MeetingDetailView`'s
    /// regenerate control) — distinct from `transcriptionQueue`'s automatic
    /// pipeline, which always uses the routing defaults. The caller already has the
    /// transcript text loaded (the detail view loads it for display), so there's no
    /// background transcription step here — just the same accessQueue-serialized FFI
    /// call every other coreStore caller uses.
    /// The calendar title check's key, re-read at the moment of the core call (the routing
    /// config was built earlier): a toggle-off, removed key or Local-only switch since then
    /// must stop it. Called from the slow lane; nothing on main waits on that queue.
    func freshTitleCheckKey() -> String? {
        let read = { self.meetingTitleCheck.activeKey(localOnly: self.routing.localOnlyMode) }
        return Thread.isMainThread ? read() : DispatchQueue.main.sync(execute: read)
    }

    func regenerateMeetingSummary(
        meetingId: Int64,
        notes: String,
        transcript: String,
        ollamaModel: String,
        routing: CoreRoutingConfig,
        customInstructions: String,
        completion: @escaping (Result<CoreMeetingSummary, CoreLLMError>?) -> Void
    ) {
        guard let slow = slowStore else {
            completion(nil)
            return
        }
        regeneratingSummaryIds.insert(meetingId)
        CoreMemoryStore.slowLaneQueue.async { [weak self] in
            let slides = slow.meetingSlidesText(meetingId: meetingId)
            // The stored segments with speaker names (as the first summary used); the caller's text without them.
            let named = slow.meetingTranscriptText(meetingId: meetingId) ?? ""
            let text = named.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? transcript : named
            let outcome = slow.summarizeMeeting(meetingId: meetingId, notes: notes, transcript: text, slides: slides, model: ollamaModel, routing: routing.withTypesafeKey(self?.freshTitleCheckKey()), customInstructions: customInstructions)
            DispatchQueue.main.async {
                self?.regeneratingSummaryIds.remove(meetingId)
                self?.recordSummaryOutcome(outcome, meetingId: meetingId)
                completion(outcome)
            }
        }
    }

    /// FR-MTG-1/2: starts capture for a specific calendar-triggered or heuristically
    /// detected meeting (accepted prompt, or auto-start) and records a `meeting` row —
    /// tagged with the matching trigger — so the session can be completed (end time +
    /// audio path) when the meeting ends. A no-op if a meeting is already active — this
    /// doesn't retroactively attach the in-progress session to the meeting. Independent
    /// of activity capture's state (FR-MTG-14). Only `.away` overrides the row's trigger;
    /// otherwise it follows the meeting's source (calendar or detected).
    func startCapture(for meeting: QualifyingMeeting, trigger: CoreMeetingTrigger = .calendar) {
        guard activeMeetingId == nil, !isStartingMeeting, hasRoomToRecord() else { return }
        // The first thing Whimbrel may need to warn about, so this is when it asks. No-op once answered.
        permissions.request(.notifications)
        // Set immediately (synchronously, before the async insertMeeting round-trip
        // below) so this guard still blocks a rapid second call during that gap —
        // `activeMeetingId` itself isn't set until the round-trip completes.
        isStartingMeeting = true
        holdPendingPromptForSplit(startingId: meeting.id)
        captureGeneration += 1
        let thisGeneration = captureGeneration
        // Start recording immediately rather than waiting on the DB round-trip below —
        // `insertMeeting` shares a serial queue with meeting-summary generation, which
        // can run for minutes on a long transcript, and audio.start() has no dependency
        // on the meeting row or its id. If a stop lands before the row lands, endMeeting()
        // already calls audio.stop() unconditionally on any captureGeneration bump, so
        // this doesn't leave an orphaned recording running.
        Task { await audio.start() }
        let startMs = Int64(meeting.startDate.timeIntervalSince1970 * 1000)
        // The new part starts now, even for a calendar meeting whose row keeps the scheduled start.
        let partStartMs = Int64(Date().timeIntervalSince1970 * 1000)
        let continueKey = MeetingContinuation.key(for: meeting)
        let calendarFallback = lastEndedCalendarMeeting.flatMap {
            MeetingContinuation.calendarFallbackMatches(lastEnded: $0, detected: meeting, now: Date()) ? $0 : nil
        }
        let newMeeting = CoreNewMeeting(
            deviceId: deviceId,
            startUnixMs: startMs,
            endUnixMs: nil,
            title: meeting.title,
            attendees: meeting.attendees,
            trigger: trigger == .away ? .away : (meeting.source == .calendar || meeting.source == .googleCalendar) ? .calendar : .detected,
            audioPath: nil,
            status: .recording
        )
        guard let store = coreStore else {
            finishStartingCapture(for: meeting, insertedMeetingId: nil, partIndex: nil, partStartMs: partStartMs, generation: thisGeneration)
            return
        }
        CoreMemoryStore.accessQueue.async {
            // A restart of the same meeting within the window continues it (reads only here).
            var previous = MeetingContinuation.findPrevious(key: continueKey, nowUnixMs: partStartMs, store: store)
            var inheritedEvent: MeetingContinuation.CalendarEvent?
            if previous == nil, let calendarFallback,
               let found = MeetingContinuation.findPrevious(key: "cal:\(calendarFallback.event.id)", nowUnixMs: partStartMs, store: store) {
                previous = found
                inheritedEvent = calendarFallback.event
            }
            if let previous {
                DispatchQueue.main.async { [weak self] in
                    self?.continueCapture(for: meeting, previous: previous, calendarEvent: inheritedEvent,
                                          partStartMs: partStartMs, newMeeting: newMeeting, continueKey: continueKey,
                                          generation: thisGeneration)
                }
                return
            }
            let insertedId = store.insertMeeting(newMeeting)
            if let insertedId, !meeting.attendeeDetails.isEmpty { store.setMeetingAttendeeDetails(meetingId: insertedId, attendees: meeting.attendeeDetails) }
            let partIndex = insertedId.flatMap { store.beginMeetingPart(meetingId: $0, startUnixMs: partStartMs, continueKey: continueKey) }
            DispatchQueue.main.async { [weak self] in
                self?.finishStartingCapture(for: meeting, insertedMeetingId: insertedId, partIndex: partIndex, partStartMs: partStartMs, generation: thisGeneration)
            }
        }
    }

    /// The continue branch of `startCapture(for:)`. The lookup only read, so a start that
    /// lost the race has nothing to undo and the old meeting stays completed. Otherwise
    /// reopen it and add the next part (its leftover live rows are dropped once the start
    /// wins, in `finishStartingCapture`). If the part row can't be written, the reopen is
    /// undone and this start becomes a new meeting (`newMeeting`, `continueKey`), so the
    /// recording never lands without a part index on the old meeting.
    /// `calendarEvent` is set when a detected start continues a calendar meeting, so that
    /// meeting's next end is again remembered against its event.
    private func continueCapture(for meeting: QualifyingMeeting, previous: MeetingContinuation.Previous,
                                 calendarEvent: MeetingContinuation.CalendarEvent?, partStartMs: Int64,
                                 newMeeting: CoreNewMeeting, continueKey: String?, generation: Int) {
        guard generation == captureGeneration, let store = coreStore else {
            isStartingMeeting = false
            return
        }
        let id = previous.meeting.id
        CoreMemoryStore.accessQueue.async {
            store.reopenMeeting(id: id)
            guard let partIndex = store.beginMeetingPart(meetingId: id, startUnixMs: partStartMs, continueKey: nil) else {
                store.undoMeetingReopen(meetingId: id, partIndex: 0)
                MeetingContinuation.log.error("meeting \(id): could not add a part; starting a new meeting instead")
                let insertedId = store.insertMeeting(newMeeting)
                if let insertedId, !meeting.attendeeDetails.isEmpty { store.setMeetingAttendeeDetails(meetingId: insertedId, attendees: meeting.attendeeDetails) }
                let newPart = insertedId.flatMap { store.beginMeetingPart(meetingId: $0, startUnixMs: partStartMs, continueKey: continueKey) }
                DispatchQueue.main.async { [weak self] in
                    self?.finishStartingCapture(for: meeting, insertedMeetingId: insertedId, partIndex: newPart, partStartMs: partStartMs, generation: generation)
                }
                return
            }
            DispatchQueue.main.async { [weak self] in
                self?.finishStartingCapture(for: meeting, insertedMeetingId: id, partIndex: partIndex,
                                            continuing: previous, calendarEvent: calendarEvent, partStartMs: partStartMs, generation: generation)
            }
        }
    }

    /// A start abandoned after its row was written (a stop or newer start came in during the
    /// round-trip): complete the row and its part with no audio so neither sits at recording.
    private func completeAbandonedStart(meetingId: Int64?, partIndex: Int64?) {
        guard let meetingId, let store = coreStore else { return }
        let endMs = Int64(Date().timeIntervalSince1970 * 1000)
        CoreMemoryStore.accessQueue.async {
            if let partIndex {
                store.completeMeetingPart(meetingId: meetingId, partIndex: partIndex, endUnixMs: endMs, audioPath: nil)
            }
            store.completeMeeting(id: meetingId, endUnixMs: endMs, audioPath: nil)
        }
    }

    /// `generation` is the value `captureGeneration` held at the moment this specific
    /// `startCapture` call began. If a stop (or a newer start) happened during the
    /// async `insertMeeting` round-trip above, `captureGeneration` will have moved on
    /// by the time this runs — in that case this continuation must NOT resume capture
    /// or overwrite state a later call already owns. If it did insert a meeting row for
    /// this now-abandoned start, mark that row complete immediately (no audio was ever
    /// attached to it) so it doesn't linger stuck at `.recording` forever.
    /// `continuing` is the earlier meeting this start continues: its stored title, notes,
    /// start and snapshots folder are kept.
    private func finishStartingCapture(for meeting: QualifyingMeeting, insertedMeetingId: Int64?, partIndex: Int64?,
                                       continuing previous: MeetingContinuation.Previous? = nil,
                                       calendarEvent inheritedEvent: MeetingContinuation.CalendarEvent? = nil,
                                       partStartMs: Int64, generation: Int) {
        isStartingMeeting = false
        guard generation == captureGeneration else {
            if let previous, let store = coreStore {
                // Lost after reopening: put the earlier meeting back exactly as it was.
                let id = previous.meeting.id
                CoreMemoryStore.accessQueue.async { store.undoMeetingReopen(meetingId: id, partIndex: partIndex ?? 0) }
            } else {
                completeAbandonedStart(meetingId: insertedMeetingId, partIndex: partIndex)
            }
            return
        }
        if let previous, let store = coreStore {
            // The start won: drop the continued meeting's leftover live rows. Queued before
            // `activeMeetingId` is set, so the new part's live writes land after this.
            let id = previous.meeting.id
            CoreMemoryStore.accessQueue.async { store.clearLiveSegments(meetingId: id) }
        }
        let title = previous?.meeting.title ?? meeting.title
        activeMeetingId = insertedMeetingId
        activeMeetingStartedAt = insertedMeetingId == nil ? nil : previous.map { Date(timeIntervalSince1970: TimeInterval($0.meeting.startUnixMs) / 1000) }
            ?? Date(timeIntervalSince1970: TimeInterval(partStartMs) / 1000)
        activeMeetingTitle = title
        activeMeetingIsContinued = previous != nil
        activePartIndex = partIndex
        activeMeetingBundleID = meeting.appBundleID
        activeCalendarEvent = (meeting.source == .calendar || meeting.source == .googleCalendar)
            ? .init(id: meeting.id, conferenceLink: meeting.conferenceLink, scheduledEnd: meeting.endDate)
            : inheritedEvent
        activeCallLink = meeting.conferenceLink ?? activeCalendarEvent?.conferenceLink
        meetingNotes = previous.map {
            MeetingContinuation.notes(stored: $0.meeting.notes, lastEnded: lastEndedMeetingNotes, meetingId: $0.meeting.id)
        } ?? ""
        // Heuristically-detected meetings only — a calendar meeting already has a real
        // attendee list, and a manual meeting has no app/browser context to poll at all.
        // `isGoogleMeet` comes from the detected meeting's own conference link, which
        // `MeetingHeuristicDetector` sets only on its browser path, from the URL it
        // matched. It is stable for the meeting's life, unlike
        // `browserMeetingWindow(forMeetingId:)`, a live snapshot that `poll()` clears and
        // re-sets during grace-period churn. A Teams or Zoom web meeting has no browser
        // participant reader yet, so it gets no participant count.
        if meeting.source == .detected, let insertedMeetingId, let store = coreStore {
            let isGoogleMeet = meeting.conferenceLink?.provider == .googleMeet
            let meetingKey = meeting.id
            participantCountEstimator = ParticipantCountEstimator(
                meetingId: insertedMeetingId,
                appBundleID: meeting.appBundleID,
                isGoogleMeet: isGoogleMeet,
                browserWindowProvider: { [detector = meetingHeuristicDetector] in
                    // Falls back to a fresh resolve if the id-keyed lookup has gone
                    // stale (a >15s tab switch let `poll()`'s grace period clear the
                    // tracked window and mint a new meetingId this closure's captured
                    // `meetingKey` can no longer match) — see
                    // `MeetingHeuristicDetector.resolveFrontmostGoogleMeetWindow()`.
                    detector.browserMeetingWindow(forMeetingId: meetingKey)
                        ?? MeetingHeuristicDetector.resolveFrontmostGoogleMeetWindow()
                },
                permissions: permissions,
                store: store
            )
            participantCountEstimator?.start()
        }
        if let insertedMeetingId {
            let meetingKey = meeting.id
            let isGoogleMeet = meeting.conferenceLink?.provider == .googleMeet
            // A continued meeting keeps its folder (if it still exists) and the name inputs it had.
            let existingFolder = previous?.snapshotsDir
                .map { URL(fileURLWithPath: $0, isDirectory: true) }
                .flatMap { FileManager.default.fileExists(atPath: $0.path) ? $0 : nil }
            let start = previous.map { Date(timeIntervalSince1970: TimeInterval($0.meeting.startUnixMs) / 1000) } ?? meeting.startDate
            // The check-in counts this recording's time (a late-accepted calendar meeting must
            // not prompt early), except a continuation, which counts from the original start.
            startCheckIn(startedAt: previous == nil ? Date(timeIntervalSince1970: TimeInterval(partStartMs) / 1000) : start)
            startMeetingSnapshots(
                meetingId: insertedMeetingId, start: start, title: title,
                appBundleID: meeting.appBundleID, provider: meeting.conferenceLink?.provider,
                existingFolder: existingFolder,
                browserWindowProvider: { [detector = meetingHeuristicDetector] in
                    detector.browserMeetingWindow(forMeetingId: meetingKey)
                        ?? (isGoogleMeet ? MeetingHeuristicDetector.resolveFrontmostGoogleMeetWindow() : nil)
                }
            )
        }
        let scheduledEndUnixMs = (meeting.source == .calendar || meeting.source == .googleCalendar) ? Int64(meeting.endDate.timeIntervalSince1970 * 1000) : nil
        meetingEndDetector.start(
            scheduledEndUnixMs: scheduledEndUnixMs,
            appBundleID: meeting.appBundleID,
            browserMeetingWindow: meetingHeuristicDetector.browserMeetingWindow(forMeetingId: meeting.id),
            isAudioActive: { [weak self] in
                guard let self else { return false }
                return Date().timeIntervalSince(self.audio.lastNonSilentAt) < 2
            },
            suppressMaxDuration: checkInSuppressesMaxDuration,
            onEnded: { [weak self] reason in self?.endActiveMeetingIfNeeded(reason: reason) }
        )
    }

    /// FR-MTG-5: a meeting can be left stuck at `.recording` forever if the app quit,
    /// crashed, or was replaced (a fresh build copied over /Applications) mid-session —
    /// the in-memory `activeMeetingId` that would normally drive `stopCapture()` is
    /// gone once that process is gone, so nothing else ever marks the row complete.
    /// Runs once at launch, before this process has created any meeting row of its
    /// own, so it can never touch a session this launch is about to start — anything
    /// still `.recording` at this point is unconditionally from a prior process life.
    /// The recording is usually still on disk (spec §6): a meeting whose open part has no
    /// audio is re-linked to the unreferenced `system-`/`mic-audio-<stamp>.caf` stamped
    /// within 30 min of that part's start, completed, and transcribed. Meetings the quit
    /// path completed (`endMeetingForQuit`) are transcribed here too, once.
    ///
    /// The transcription hop to main runs after `init` returns, so `transcriptionQueue`
    /// (lazy) is created on that first use; nothing else enqueues these meetings at launch.
    private func reconcileOrphanedRecordingMeetings() {
        guard let store = coreStore else { return }
        // Consumed now, so a meeting whose transcription fails is not retried every launch
        // ("Transcribe again" still offers it).
        let quitEnded = Set(UserDefaults.standard.array(forKey: Self.pendingLaunchTranscriptionKey) as? [Int64] ?? [])
        UserDefaults.standard.removeObject(forKey: Self.pendingLaunchTranscriptionKey)
        CoreMemoryStore.accessQueue.async { [weak self] in
            // Provisional live-transcript rows nothing can use any more (older than a day,
            // orphaned, or replaced by the real rows).
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
            store.purgeStaleLiveSegments(nowUnixMs: nowMs)
            let recent = store.recentMeetings(limit: 200)
            var jobs: [MeetingTranscriptionQueue.Job] = []
            var referenced: Set<String>?
            let files = (try? FileManager.default.contentsOfDirectory(atPath: SpikePaths.root.path)) ?? []
            for meeting in recent where meeting.status == .recording {
                let open = StrandedAudioMatch.openPart(store.meetingParts(meetingId: meeting.id) ?? [])
                // A continued meeting keeps part 1's path on the row, so ask the open part.
                let hasAudio = (open.map(\.audioPath) ?? meeting.audioPath) != nil
                if referenced == nil, !hasAudio {
                    referenced = StrandedAudioMatch.referencedNames(paths: recent.flatMap { m in
                        [m.audioPath] + (store.meetingParts(meetingId: m.id) ?? []).map(\.audioPath)
                    })
                }
                let start = Date(timeIntervalSince1970: Double(open?.startUnixMs ?? meeting.startUnixMs) / 1000)
                guard !hasAudio, let name = StrandedAudioMatch.audioFileName(meetingStart: start, fileNames: files,
                                                                              referenced: referenced ?? []) else {
                    store.completeMeeting(id: meeting.id, endUnixMs: nowMs, audioPath: meeting.audioPath)
                    // Audio already linked: transcribe it unless that part already landed.
                    if let linked = open?.audioPath ?? meeting.audioPath, !(open?.transcribed ?? false) {
                        MeetingContinuation.log.notice("meeting \(meeting.id): completed interrupted recording, transcribing its linked audio")
                        jobs.append(.init(meetingId: meeting.id, partIndex: open?.partIndex ?? 1, audioPath: linked, notes: meeting.notes ?? ""))
                    } else {
                        MeetingContinuation.log.notice("meeting \(meeting.id): completed interrupted recording with no audio to transcribe")
                    }
                    continue
                }
                let url = SpikePaths.audioFile(name: name)
                referenced?.formUnion(StrandedAudioMatch.referencedNames(paths: [url.path]))
                let endMs = StrandedAudioMatch.recordingEnd(of: url).map { Int64($0.timeIntervalSince1970 * 1000) } ?? nowMs
                MeetingContinuation.completeRecording(store: store, meetingId: meeting.id, partIndex: open?.partIndex,
                                                      continued: false, endUnixMs: endMs, audioPath: url.path)
                MeetingContinuation.log.notice("meeting \(meeting.id): re-linked stranded recording \(name, privacy: .public)")
                jobs.append(.init(meetingId: meeting.id, partIndex: open?.partIndex ?? 1, audioPath: url.path, notes: meeting.notes ?? ""))
            }
            for id in quitEnded where !jobs.contains(where: { $0.meetingId == id }) {
                guard let meeting = store.getMeeting(id: id), meeting.status == .completed,
                      let end = meeting.endUnixMs, nowMs - end < 86_400_000 else {
                    MeetingContinuation.log.notice("meeting \(id): quit-marked but not completed or older than a day; not transcribed (Transcribe again can)")
                    continue
                }
                let open = StrandedAudioMatch.openPart(store.meetingParts(meetingId: id) ?? [])
                let transcribed = open.map(\.transcribed) ?? (meeting.transcriptPath != nil)
                guard !transcribed, let path = open.map(\.audioPath) ?? meeting.audioPath else {
                    MeetingContinuation.log.notice("meeting \(id): quit-marked; already transcribed or no audio, nothing to do")
                    continue
                }
                jobs.append(.init(meetingId: id, partIndex: open?.partIndex ?? 1, audioPath: path, notes: meeting.notes ?? ""))
            }
            guard !jobs.isEmpty else { return }
            DispatchQueue.main.async {
                guard let self else { return }
                for job in jobs where job.meetingId != self.processingMeetingId {
                    self.transcribePart(meetingId: job.meetingId, partIndex: job.partIndex, audioPath: job.audioPath, notes: job.notes)
                }
            }
        }
    }
}

extension AppState {
    /// Which device a row came from (docs/superpowers/specs/2026-07-30-timeline-device-attribution-design.md).
    /// Content sync merges peer devices' rows into the same store, so every surface
    /// that shows a row labels it the same way: the device's synced name, "This Mac",
    /// or a short id for a device this Mac hasn't seen a name for.
    func deviceLabel(for deviceId: String) -> String {
        if let matched = syncState.devices.first(where: { $0.publicKey == deviceId }),
           let name = DeviceDisplayName.name(matched.name, platform: matched.platform) {
            return name
        }
        if deviceId == self.deviceId {
            return String(localized: "This Mac")
        }
        if deviceId == "spike-device" {
            return String(localized: "Unknown device")
        }
        return String(localized: "Device \(deviceId.prefix(6))")
    }
}

// MARK: - Away Mode

extension AppState {
    /// Joining reads Zoom's window (`ZoomMeetingReader`) to confirm the mic and camera are off.
    /// On in all builds since the manual pass (docs/superpowers/plans/2026-10-27-away-mode-manual-pass.md).
    static let awayZoomJoinEnabled = true

    /// The live `AwayEnvironment`: this Mac, Zoom, and AppState's own capture.
    final class LiveAwayEnvironment: AwayEnvironment {
        private unowned let app: AppState
        private let reader: ZoomStateReading = LiveZoomStateReader()
        let chat: ZoomChatPosting = ZoomChatPoster()
        let leaver: ZoomLeaving = ZoomLeaver()
        /// The meeting this environment started, so `endCapture` never ends another recording.
        private var meetingId: Int64?

        init(app: AppState) { self.app = app }

        var now: Date { Date() }
        var isScreenLocked: Bool {
            let session = CGSessionCopyCurrentDictionary() as? [String: Any]
            return session?["CGSSessionScreenIsLocked"] as? Bool ?? false
        }
        var isMicInUse: Bool { MeetingHeuristicDetector.isMicrophoneInUse() }
        var isRecording: Bool {
            app.activeMeetingId != nil || app.isStartingMeeting || app.audio.micActive || app.audio.systemAudioActive
        }
        var profileName: String { app.profile.name }

        func open(_ url: URL) -> Bool {
            guard AppState.awayZoomJoinEnabled else { return false }
            return NSWorkspace.shared.open(url)
        }

        func zoomState() -> ZoomWindowState { reader.currentState() }

        /// `startCapture(for:)` returns before its row lands; wait for this start to finish.
        /// A refused or superseded start (the generation moved on) is nil.
        func startCapture(_ m: QualifyingMeeting) async -> Int64? {
            let generation = app.captureGeneration + 1
            app.startCapture(for: m, trigger: .away)
            while app.isStartingMeeting, app.captureGeneration == generation {
                try? await Task.sleep(nanoseconds: 250_000_000)
            }
            guard app.captureGeneration == generation else { return nil }
            meetingId = app.activeMeetingId
            return meetingId
        }

        func endCapture() {
            if let meetingId, app.activeMeetingId == meetingId { app.endActiveMeetingIfNeeded() }
            meetingId = nil
        }

        func insertMissedMeeting(_ m: QualifyingMeeting) async -> Int64? {
            guard let store = app.coreStore else { return nil }
            let row = CoreNewMeeting(
                deviceId: app.deviceId,
                startUnixMs: Int64(m.startDate.timeIntervalSince1970 * 1000),
                endUnixMs: Int64(m.endDate.timeIntervalSince1970 * 1000),
                title: m.title,
                attendees: m.attendees,
                trigger: .away,
                audioPath: nil,
                status: .completed
            )
            let details = m.attendeeDetails
            return await AppState.onAccessQueue {
                let id = store.insertMeeting(row)
                if let id, !details.isEmpty { store.setMeetingAttendeeDetails(meetingId: id, attendees: details) }
                return id
            }
        }

        func setOutcome(meetingId: Int64, outcome: AwayOutcome, at: Date) async {
            guard let store = app.coreStore else { return }
            _ = await AppState.onAccessQueue { store.setMeetingAwayOutcome(meetingId: meetingId, outcome: outcome, at: at) }
        }

        func recordFollowup(_ req: AwayFollowupRequest) async {
            guard let store = app.coreStore else { return }
            _ = await AppState.onAccessQueue { store.recordAwayFollowup(req) }
        }

        func sleep(seconds: Double) async {
            try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
        }
    }
}

enum RestartBlock { case recording, missing }

extension AppState {
    /// Shared by the "Update ready" banner and Settings → General. Returns why it did not
    /// restart, or nil when the relaunch helper was started (or a failure was only logged).
    func restartIntoUpdate(_ update: UpdateChecker.DownloadedUpdate) -> RestartBlock? {
        guard activeMeetingId == nil else { return .recording }
        // The staged update lives under FileManager.default.temporaryDirectory (see
        // UpdateChecker.download()), which macOS's periodic cleaner can reap during a
        // long-running session. A user could click "Restart Now" days after seeing
        // "Later" on a path that no longer exists, which would otherwise make the
        // relaunch helper's `ditto` fail mid-swap, after the old app is already renamed
        // aside. Bail out before spawning anything or terminating this process.
        guard FileManager.default.fileExists(atPath: update.appPath.path) else {
            Logger(subsystem: "com.mpointer.kestrel", category: "shell").error("restartIntoUpdate: staged update at \(update.appPath.path, privacy: .public) no longer exists, aborting restart")
            return .missing
        }
        guard let helperPath = Bundle.main.path(forResource: "RelaunchHelper", ofType: "sh") else { return nil }
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
            Logger(subsystem: "com.mpointer.kestrel", category: "shell").error("restartIntoUpdate: failed to launch RelaunchHelper: \(error.localizedDescription, privacy: .public)")
        }
        return nil
    }
}

/// Recording notices shown in notifications, from the String Catalog.
enum StatusMenuCopy {
    static var lowDiskStart: String { String(localized: "Whimbrel didn't start recording: less than 5 GB free on this Mac.") }
    static var checkInWarning: String { String(localized: "Still recording? Whimbrel stops in 5 minutes unless you choose Keep recording.") }
    static func forcedStop(durationCap: Bool, saved: Bool) -> String {
        switch (durationCap, saved) {
        case (true, true): String(localized: "Whimbrel stopped recording: 6-hour limit reached. The recording was saved.")
        case (true, false): String(localized: "Whimbrel stopped recording: 6-hour limit reached.")
        case (false, true): String(localized: "Whimbrel stopped recording: less than 2 GB free. The recording was saved.")
        case (false, false): String(localized: "Whimbrel stopped recording: less than 2 GB free.")
        }
    }
}

// MARK: - Storage (Settings → Storage, low-disk notice)

extension AppState {
    /// Settings → Capture & privacy, where Storage lives, with Free up space showing.
    func openFreeUpSpace() {
        pendingSettingsSection = .capture
        showFreeUpSpace = true
        NSApp.activate(ignoringOtherApps: true)
        openSettingsWindow?()
    }

    /// At launch, then daily: warns (once a day) when the data folder's or the media folder's
    /// volume has under 10 GB free. Warns only; recording isn't blocked.
    func startLowDiskChecks() {
        let check = { [weak self] in
            guard let self else { return }
            let media = self.mediaSettings.folderURL
            DispatchQueue.global(qos: .utility).async {
                let low = StorageFootprint.lowDiskVolumes(dataRoot: SpikePaths.root, mediaFolder: media)
                guard !low.isEmpty else { return }
                Self.lowDiskLog.notice("low disk: \(low.map { "\($0.name) \($0.freeBytes)" }.joined(separator: ", "), privacy: .public)")
                LowDiskNotice.postIfDue(low)
            }
        }
        check()
        lowDiskTimer = Timer.scheduledTimer(withTimeInterval: 24 * 60 * 60, repeats: true) { _ in check() }
    }

    fileprivate static let lowDiskLog = Logger(subsystem: "com.mpointer.kestrel", category: "storage")
}

// MARK: - Hosted model defaults (core/api::model_defaults)

extension AppState {
    /// Puts the store's cached defaults in force and re-reads the hosted models against
    /// them, then fetches defaults.json a minute after launch and once a day. The fetch
    /// holds a store for up to about 20 s, so it runs on a connection of its own, off both
    /// store lanes. A user's pick clears core's "isn't available" note for that provider.
    func startModelDefaults() {
        guard let store = coreStore else { return }
        routing.hostedModelChosen = { [weak self] provider in
            CoreMemoryStore.accessQueue.async {
                store.clearModelUnavailable(provider: provider)
                DispatchQueue.main.async { self?.refreshModelNotes() }
            }
        }
        CoreMemoryStore.accessQueue.async { [weak self] in
            store.loadModelDefaults()
            let notes = store.modelUnavailableNotes()
            DispatchQueue.main.async {
                self?.routing.resolveHostedDefaults()
                self?.routing.modelUnavailableNotes = notes
            }
        }
        modelDefaultsTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(60))
            while !Task.isCancelled {
                let refreshed = await Self.refreshModelDefaultsOffLanes(primary: store)
                if refreshed { self?.routing.resolveHostedDefaults() }
                try? await Task.sleep(for: .seconds(24 * 60 * 60))
            }
        }
    }

    /// Re-reads core's "isn't available" notes for Settings → AI Providers.
    func refreshModelNotes() {
        guard let store = coreStore else { return }
        CoreMemoryStore.accessQueue.async { [weak self] in
            let notes = store.modelUnavailableNotes()
            DispatchQueue.main.async { self?.routing.modelUnavailableNotes = notes }
        }
    }

    /// One defaults.json fetch on a short-lived connection of its own, so neither store
    /// lane waits on the network. True when a usable file was cached and put in force.
    private nonisolated static func refreshModelDefaultsOffLanes(primary: CoreMemoryStore) async -> Bool {
        let own = CoreMemoryStore.accessQueue.sync { CoreMemoryStore(secondaryTo: primary, path: SpikePaths.database) }
        guard let own else { return false }
        return await Task.detached(priority: .utility) { own.refreshModelDefaults() }.value
    }
}
