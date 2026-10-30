import AppKit
import ApplicationServices
import Combine
import Foundation
import os
import UserNotifications

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
    /// The shell's selected tab, here rather than in `WhimbrelShellView` so other screens
    /// (Today's "Review" button) can switch tabs.
    @Published var selectedTab: WhimbrelTab = .today
    /// Set by the ⌘F and ⌘N menu commands and cleared by Library once it acts on them.
    /// Flags rather than events, so a request made before Library is on screen still
    /// lands when it appears.
    @Published var pendingLibrarySearch = false
    @Published var pendingNewConversation = false
    /// Text from Today's composer, sent as the first message of the new conversation.
    @Published var pendingAskPrompt: String?

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

    /// The connector keeps every future occurrence of a recurring meeting (for years
    /// ahead), while EventKit covers only the rest of today, so the connector's list is
    /// cut to the same window before merging.
    nonisolated static func todaysMeetings(eventKit: [QualifyingMeeting], oauth: [QualifyingMeeting], now: Date) -> [QualifyingMeeting] {
        let end = CalendarService.todayWindowEnd(now: now)
        return CoreMemoryStore.mergeMeetings(eventKit: eventKit, oauth: oauth.filter { $0.startDate < end })
    }
    let meetingTrigger = MeetingTriggerService()
    /// "Away Mode": join Zoom meetings unattended. Owns the keep-awake assertion while on.
    let awayMode = AwayModeSettings(keepAwake: KeepAwake())
    /// Away Mode's join logic. Eligible calendar prompts go here instead of the alert panel.
    private(set) lazy var awayCoordinator = AwayModeCoordinator(
        env: LiveAwayEnvironment(app: self), isEnabled: { [awayMode] in awayMode.enabled }
    )
    let meetingHeuristicDetector = MeetingHeuristicDetector()
    let meetingEndDetector = MeetingEndDetector()
    let meetingAlertPanel = MeetingAlertPanelController()
    let retention = RetentionSettings()
    let retentionScheduler = RetentionScheduler()
    let routineScheduler = RoutineScheduler()
    /// Task 7: spawns/stops the real `whimbrel-mcp-server` binary, driven by the "Enable
    /// MCP Server" Settings toggle (`mcpServerProcess.enabled`, wired to `start()`/`stop()`
    /// below in `init`, same pattern as `meetingTrigger.$heuristicDetectionEnabled`).
    let mcpServerProcess = McpServerProcessManager()
    var routing = RoutingSettings()
    let replyPriority = ReplyPrioritySettings()
    let meetingPrepSettings = MeetingPrepSettings()
    let meetingTitleCheck = MeetingTitleCheckSettings()
    /// "Live transcript during meetings" (on by default) and the service that transcribes a
    /// recording in chunks while it is being written
    /// (docs/superpowers/specs/2026-10-06-live-transcription-design.md).
    let liveTranscriptSettings = LiveTranscriptSettings()
    /// "Ask if I'm still recording after N hours" and its panel
    /// (docs/superpowers/specs/2026-10-24-recording-failsafes-design.md §2).
    let recordingCheckInSettings = RecordingCheckInSettings()
    let recordingCheckInPanel = RecordingCheckInPanelController()
    /// The active meeting's check-in, polled every 5 s by `checkInTimer`; nil when no meeting.
    private var activeCheckIn: RecordingCheckIn?
    private var checkInTimer: Timer?
    /// "Meeting snapshots": slide images from the meeting window (on by default).
    let snapshotSettings = SnapshotSettings()
    /// System / Light / Dark, per device.
    let appearanceSettings = AppearanceSettings()
    /// The Settings skin chat. Owned here, not by its view, so the chat survives the Settings
    /// window rebuild (`ThemedRoot`) that every skin it switches on causes.
    lazy var skinChat = SkinChatSession(
        skinStore: .shared, userSkins: .shared, backend: CoreSkinTurnBackend(routing: routing), routing: routing
    )
    lazy var liveTranscript = LiveTranscriptionService(
        store: { [weak self] in self?.coreStore },
        modelPath: { TranscriptionService.modelPath.path },
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
            store?.setDeviceId(ownDeviceId)
        }
        let slow = slowStore
        CoreMemoryStore.slowLaneQueue.async { slow?.setDeviceId(ownDeviceId) }
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
        syncState.resetPushCursor = { [store = coreStore] in
            await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
                CoreMemoryStore.accessQueue.async {
                    continuation.resume(returning: store?.resetSyncPushCursor() ?? false)
                }
            }
        }
        reconcileOrphanedRecordingMeetings()
        // `queue: nil` runs the block synchronously on the posting (main) thread, before
        // `terminate()` exits; see McpServerProcessManager.
        terminationObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.willTerminateNotification, object: nil, queue: nil
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.endMeetingForQuit() }
        }

        permissions.objectWillChange
            .merge(with: audio.objectWillChange, screenText.objectWillChange)
            .merge(with: calendar.objectWillChange, meetingTrigger.objectWillChange, googleCalendarPolling.objectWillChange)
            .merge(with: retention.objectWillChange, meetingHeuristicDetector.objectWillChange)
            .merge(with: routing.objectWillChange, syncState.objectWillChange, replyPriority.objectWillChange, meetingPrepSettings.objectWillChange, meetingTitleCheck.objectWillChange, liveTranscriptSettings.objectWillChange, appearanceSettings.objectWillChange)
            .merge(with: snapshotSettings.objectWillChange, recordingCheckInSettings.objectWillChange, awayMode.objectWillChange)
            .merge(with: settingsSyncCoordinator.objectWillChange, contentSyncCoordinator.objectWillChange)
            .merge(with: updateChecker.objectWillChange)
            .merge(with: promptLibrary.objectWillChange, profile.objectWillChange, profileSuggestions.objectWillChange, routineHost.objectWillChange)
            .merge(with: calendarConnectorState.objectWillChange)
            .merge(with: gmailConnectorState.objectWillChange)
            .merge(with: outlook365ConnectorState.objectWillChange)
            .merge(with: retentionScheduler.objectWillChange)
            .merge(with: routineScheduler.objectWillChange)
            .merge(with: mcpServerProcess.objectWillChange)
            .sink { [weak self] _ in self?.objectWillChange.send() }
            .store(in: &cancellables)

        // Live transcript follows the active meeting: it starts when a recording becomes the
        // active meeting (if the setting is on) and stops when it ends. It reads the recording
        // files lazily, so it does not matter that audio starts before the meeting row exists.
        $activeMeetingId
            .removeDuplicates()
            .sink { [weak self] id in
                guard let self else { return }
                if let id, self.liveTranscriptSettings.enabled {
                    self.liveTranscript.start(meetingId: id)
                } else {
                    self.liveTranscript.stop()
                }
            }
            .store(in: &cancellables)

        // The setting is a privacy control, so it applies at once in either direction: turning
        // it off during a meeting stops live transcription and deletes all live text already
        // stored; turning it on during one starts it.
        liveTranscriptSettings.$enabled
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
            let why = stop == .durationCap ? "6-hour limit reached." : "less than 2 GB free."
            self.postRecordingNotice("Whimbrel stopped recording: \(why)" + (hasMeeting ? " The recording was saved." : ""))
        }
        calendar.start()
        googleCalendarPolling.start(calendarConnectorState: calendarConnectorState, coreStore: coreStore)
        gmailPolling.start(gmailConnectorState: gmailConnectorState, coreStore: coreStore, deviceId: deviceId)
        meetingTrigger.start(meetingsProvider: { [weak self] in self?.allUpcomingMeetings ?? [] })
        retentionScheduler.start(retention: retention, store: coreStore, audio: audio)
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
        Task { await profileSuggestions.reload() }
        Task { await syncState.silentResumeIfPossible() }
        updateChecker.start()
        settingsSyncCoordinator.observeSyncState()
        contentSyncCoordinator.observeSyncState()
        scheduleJobsBatchStep()
        scheduleEpisodeSegmentStep()
        scheduleReplyPriorityStep()
        scheduleScreenTextIngestStep()
        scheduleTranscriptBackfillStep()

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
                if self.activeMeetingId == nil, !self.awayCoordinator.isRunning {
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

        meetingTrigger.$pendingPrompt
            .sink { [weak self] prompt in
                guard let self else { return }
                self.callWaitTimer?.invalidate()
                self.callWaitTimer = nil
                guard let prompt else {
                    self.meetingAlertPanel.hide()
                    return
                }
                if AwayModeCoordinator.takesPrompt(prompt, enabled: self.awayMode.enabled, joinAvailable: Self.awayZoomJoinEnabled) {
                    self.meetingAlertPanel.hide()
                    self.scheduleAwayRun(for: prompt)
                    return
                }
                self.promptNormally(prompt)
            }
            .store(in: &cancellables)

        calendar.$upcomingMeetings
            .merge(with: googleCalendarPolling.$oauthMeetings)
            .sink { [weak self] _ in
                DispatchQueue.main.async { self?.persistCalendarEventItems() }
            }
            .store(in: &cancellables)
    }

    /// The meeting-alert panel for a pending prompt (Away Mode off, or it handed the meeting back).
    private func promptNormally(_ prompt: QualifyingMeeting) {
        // A calendar event only auto-starts once this Mac is in the call: with
        // several Macs in one room, an event on a Mac that isn't in it would
        // otherwise record whatever its mic overhears. Until then the prompt waits,
        // and the countdown starts if the call begins while it's showing.
        let waitingForCall = self.meetingTrigger.autoStartEnabled && prompt.source == .calendar
            && !MeetingHeuristicDetector.isMicrophoneInUse()
        self.showMeetingAlert(for: prompt, autoStart: self.meetingTrigger.autoStartEnabled && !waitingForCall)
        guard waitingForCall else { return }
        self.callWaitTimer = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] timer in
            Task { @MainActor in
                guard let self, self.meetingTrigger.pendingPrompt?.id == prompt.id else {
                    timer.invalidate()
                    return
                }
                guard MeetingHeuristicDetector.isMicrophoneInUse() else { return }
                timer.invalidate()
                self.callWaitTimer = nil
                self.showMeetingAlert(for: prompt, autoStart: true)
            }
        }
    }

    /// docs/superpowers/specs/2026-08-07-items-schema-migration-design.md: persists
    /// calendar events into `items` whenever the merged calendar/Google Calendar poll
    /// changes — one integration point regardless of which calendar an event came from.
    /// `$upcomingMeetings`/`$oauthMeetings`, like `objectWillChange`, fire on `willSet` —
    /// *before* the new value lands — so reading `allUpcomingMeetings` synchronously
    /// inside the sink would see the previous poll's data. The subscription above defers
    /// the call with `DispatchQueue.main.async` so it runs after both `@Published`
    /// properties have finished mutating, and `allUpcomingMeetings` reflects the fresh
    /// poll by the time this reads it. The actual FFI call is dispatched onto
    /// `CoreMemoryStore.accessQueue` — the same shared queue every other coreStore
    /// caller in this file uses, per `CoreMemoryStore`'s not-thread-safe contract.
    private func persistCalendarEventItems() {
        let inputs = allUpcomingMeetings.compactMap { CoreMemoryStore.calendarEventItem(for: $0, deviceId: deviceId) }
        guard !inputs.isEmpty, let store = coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            store.upsertCalendarEventItems(inputs)
        }
    }

    var statusSummary: String {
        var parts: [String] = []
        if audio.systemAudioActive { parts.append("system audio") }
        if audio.micActive { parts.append("mic") }
        if screenText.isRunning { parts.append("screen text") }
        let suffix: String
        if parts.isEmpty {
            suffix = activityCaptureEnabled ? "Activity capture on — waiting for permissions" : "Activity capture paused"
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
        // Set immediately (synchronously, before the async insertMeeting round-trip
        // below) so a rapid second "Start Meeting" click during that gap is blocked —
        // `activeMeetingId` itself isn't set until the round-trip completes. Mirrors
        // `startCapture(for:)`'s identical synchronous set for the same reason.
        isStartingMeeting = true
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
            finishStartingManualCapture(insertedMeetingId: nil, partIndex: nil, start: startDate, generation: thisGeneration)
            return
        }
        CoreMemoryStore.accessQueue.async {
            let insertedId = store.insertMeeting(newMeeting)
            // Manual meetings have no continue key, so they never continue.
            let partIndex = insertedId.flatMap { store.beginMeetingPart(meetingId: $0, startUnixMs: startMs, continueKey: nil) }
            DispatchQueue.main.async { [weak self] in
                self?.finishStartingManualCapture(insertedMeetingId: insertedId, partIndex: partIndex, start: startDate, generation: thisGeneration)
            }
        }
    }

    /// Mirrors `finishStartingCapture(for:insertedMeetingId:generation:)`'s exact
    /// race-guard shape: if a newer meeting-start happened during the async insert
    /// above (`generation != captureGeneration`), the just-inserted row (if any) is
    /// immediately completed with a null `audioPath` — no audio ever started for that
    /// already-cancelled session — rather than left stuck at `.recording` forever.
    private func finishStartingManualCapture(insertedMeetingId: Int64?, partIndex: Int64?, start: Date, generation: Int) {
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
        meetingNotes = ""
        Task { await audio.start() }
        if let insertedMeetingId {
            startMeetingSnapshots(meetingId: insertedMeetingId, start: start, title: "Meeting",
                                  appBundleID: nil, provider: nil, browserWindowProvider: { nil })
            startCheckIn(startedAt: start)
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
            postRecordingNotice("Whimbrel didn't start recording: less than 5 GB free on this Mac.")
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
        recordingCheckInPanel.hide()
    }

    private func pollCheckIn() {
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
            postRecordingNotice("Still recording? Whimbrel stops in 5 minutes unless you choose Keep recording.")
        case .stop:
            recordingCheckInPanel.hide()
            endActiveMeetingIfNeeded(reason: .checkInTimeout)
        }
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
            storage: SnapshotStorage(root: snapshotSettings.rootURL,
                                     fallbackRoot: SpikePaths.root.appendingPathComponent("Snapshots")),
            store: store, existingFolder: existingFolder
        )
        if snapshotSettings.enabled { meetingSnapshotService?.start() }
    }

    /// FR-MTG-14: ends the active meeting -- audio + completion/transcription only,
    /// independent of activity capture (never touches `activityCaptureEnabled`/
    /// `screenText`).
    private func endMeeting(reason: MeetingEndDetector.EndReason?) {
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
                transcribePart(meetingId: id, partIndex: index, audioPath: audioPath, notes: notes)
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

    /// A completed meeting with a part whose recording is still on disk but has no
    /// transcript, e.g. one whose transcription was cut short by the app quitting.
    /// Checked off main (`TranscribeAgainAvailability`); false until that check lands.
    func canTranscribeAgain(_ meeting: CoreMeeting) -> Bool {
        transcribeAgainAvailability?.canTranscribeAgain(meeting) ?? false
    }

    /// Views read it through `canTranscribeAgain`; its changes re-render AppState's observers.
    private lazy var transcribeAgainAvailability: TranscribeAgainAvailability? = coreStore.map { store in
        let availability = TranscribeAgainAvailability(store: store)
        availability.objectWillChange.sink { [weak self] _ in self?.objectWillChange.send() }.store(in: &cancellables)
        return availability
    }

    /// Re-transcribes every part with audio on disk; each part's segments are replaced.
    func transcribeAgain(_ meeting: CoreMeeting) {
        guard processingMeetingId == nil, canTranscribeAgain(meeting), let transcriptionQueue else { return }
        markProcessing(meeting.id)
        transcriptionQueue.transcribeAgain(meetingId: meeting.id, notes: meeting.notes ?? "")
    }

    /// FR-MTG-5: queues one part's transcription (`MeetingTranscriptionQueue`, one part at
    /// a time, in order); the last part to land summarizes the whole meeting.
    private func transcribePart(meetingId: Int64, partIndex: Int64, audioPath: String?, notes: String) {
        guard let transcriptionQueue else { return }
        if audioPath != nil { markProcessing(meetingId) }
        transcriptionQueue.enqueue(.init(meetingId: meetingId, partIndex: partIndex, audioPath: audioPath, notes: notes))
    }

    /// The new meeting from a split waits here until the original's summary finishes.
    private var pendingSplitSummary: Int64?

    /// "Split into separate meetings": the last part becomes its own meeting; both are
    /// summarized again, one after the other.
    func splitLastPart(of meeting: CoreMeeting, onSnapshotMoveFailed: @escaping () -> Void = {}) {
        guard processingMeetingId == nil, let store = coreStore, let transcriptionQueue else { return }
        markProcessing(meeting.id)
        processingStage = "Splitting…"
        let storage = SnapshotStorage(root: snapshotSettings.rootURL, fallbackRoot: SpikePaths.root.appendingPathComponent("Snapshots"))
        MeetingLifecycle.splitLastPart(store: store, meetingId: meeting.id, makeFolder: { try storage.makeMeetingFolder(start: $0, title: $1) }) { [weak self] outcome in
            guard let self else { return }
            guard let outcome else { self.clearProcessingState(for: meeting.id); return }
            if outcome.snapshotMoveFailed { onSnapshotMoveFailed() }
            self.pendingSplitSummary = outcome.newMeetingId
            transcriptionQueue.summarizeExisting(meetingId: meeting.id, notes: meeting.notes ?? "")
        }
    }

    private func markProcessing(_ meetingId: Int64) {
        guard processingMeetingId == nil else { return }
        processingMeetingId = meetingId
        processingStage = "Transcribing…"
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
                        _ = slow?.summarizeMeeting(meetingId: meetingId, notes: notes, transcript: transcript, slides: slides, model: ollamaModel, routing: routingConfig.withTypesafeKey(self?.freshTitleCheckKey()), customInstructions: instructions)
                        DispatchQueue.main.async { self?.clearProcessingState(for: meetingId) }
                    }
                }
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
                let threadsSummarized = slow?.runThreadSummarizeBatch(model: ollamaModel, maxJobs: 1, customInstructions: threadSummaryInstructions) ?? 0
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
    /// throttled 0.1s-when-busy/30s-when-idle cadence as `scheduleJobsBatchStep()`, but
    /// deliberately WITHOUT the `PowerIdleGate` check (#52). Episodes are meant to be a
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
                let summarized = slow?.runEpisodeSummarizeBatch(model: ollamaModel, maxJobs: 1, customInstructions: episodeSummaryInstructions) ?? 0
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
        CoreMemoryStore.slowLaneQueue.async { [weak self] in
            let slides = slow.meetingSlidesText(meetingId: meetingId)
            let outcome = slow.summarizeMeeting(meetingId: meetingId, notes: notes, transcript: transcript, slides: slides, model: ollamaModel, routing: routing.withTypesafeKey(self?.freshTitleCheckKey()), customInstructions: customInstructions)
            DispatchQueue.main.async {
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
        // Set immediately (synchronously, before the async insertMeeting round-trip
        // below) so this guard still blocks a rapid second call during that gap —
        // `activeMeetingId` itself isn't set until the round-trip completes.
        isStartingMeeting = true
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
                      let end = meeting.endUnixMs, nowMs - end < 86_400_000 else { continue }
                let open = StrandedAudioMatch.openPart(store.meetingParts(meetingId: id) ?? [])
                let transcribed = open.map(\.transcribed) ?? (meeting.transcriptPath != nil)
                guard !transcribed, let path = open.map(\.audioPath) ?? meeting.audioPath else { continue }
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
           let name = matched.name?.trimmingCharacters(in: .whitespacesAndNewlines), !name.isEmpty {
            return name
        }
        if deviceId == self.deviceId {
            return "This Mac"
        }
        if deviceId == "spike-device" {
            return "Unknown device"
        }
        return "Device \(deviceId.prefix(6))"
    }
}

// MARK: - Away Mode

extension AppState {
    /// Task 7b replaces this: until the live Zoom readers land, Zoom is never opened, so
    /// an Away run ends at once as "app failed" instead of joining a meeting it can't watch.
    static let awayZoomJoinEnabled = false

    /// The live `AwayEnvironment`: this Mac, Zoom, and AppState's own capture.
    final class LiveAwayEnvironment: AwayEnvironment {
        private unowned let app: AppState
        /// Task 7b replaces this.
        private let reader: ZoomStateReading = PlaceholderZoomStateReader()
        /// Task 7b replaces this.
        let chat: ZoomChatPosting = PlaceholderZoomChatPoster()
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
            return await AppState.onAccessQueue { store.insertMeeting(row) }
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

    // Task 7b replaces this
    private struct PlaceholderZoomStateReader: ZoomStateReading {
        func currentState() -> ZoomWindowState { .notOpen }
    }

    // Task 7b replaces this
    private struct PlaceholderZoomChatPoster: ZoomChatPosting {
        func post(_ text: String) async -> Bool { false }
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
