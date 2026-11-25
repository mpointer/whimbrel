import AppKit
import SwiftUI
import WhimbrelUI

/// Turns the app's state into the plain values the redesigned menu bar menu draws (docs/ui-redesign/screens/menu-bar.md).
/// Everything here is pure, so tests can check it without an `AppState`, a store or a clock.
enum MenuBarAdapter {
    // MARK: Recording

    /// Pause or Resume is offered and not waiting: the old menu's rule. Resume stays on while paused even during the
    /// restart, since `resumeRecording()` ignores a second tap itself.
    static func canTogglePause(canPause: Bool, paused: Bool) -> Bool { canPause || paused }

    /// The recording section whenever anything records or a meeting is starting (`RecordingPauseRules.holdsRecording`, the
    /// app's one answer to that), nothing otherwise. A recording state is never hidden or shortened.
    ///
    /// A recording that says it is running while neither the microphone nor system audio is live is a problem, not silence, once
    /// capture has had `WMenuRecordingRules.startGrace` seconds from `startedAt` to begin; before that the row says it is
    /// starting (audio starts in a task after the meeting is open, so every start has that gap). `audioReason` is the capture
    /// code's own reason, when it gave one. A paused recording keeps its flags set while nothing is captured, so only a running
    /// recording is judged by them. With no `startedAt` the streams are judged at once.
    static func recording(
        holdsRecording: Bool, state: WLiveState, elapsedSeconds: Int?, title: String?, canPause: Bool, paused: Bool,
        microphoneLive: Bool, systemAudioLive: Bool, audioReason: String? = nil, canStop: Bool = true, isSaving: Bool = false,
        startedAt: Date? = nil, now: Date = Date()
    ) -> WMenuRecording? {
        guard holdsRecording, var bar = state.barState else { return nil }
        bar = WMenuRecordingRules.judged(
            bar, microphoneLive: microphoneLive, systemAudioLive: systemAudioLive, startedAt: startedAt, now: now, reason: audioReason)
        return WMenuRecording(
            state: bar, elapsedSeconds: elapsedSeconds, title: title, canTogglePause: canTogglePause(canPause: canPause, paused: paused),
            streams: WMenuStreams(microphone: microphoneLive, systemAudio: systemAudioLive), canStop: canStop, isSaving: isSaving)
    }

    /// What the recording controls offer where Stop is (`WMenuRecordingRules.stopOffer`, the one rule the menu and the main window's
    /// recording bar share). Stop does something when a meeting is open, when audio records with no meeting row
    /// (`AppState.endActiveMeetingIfNeeded`), and, since #992, for a detected or calendar start whose audio is already on and whose
    /// meeting row is not saved yet (`canStopStarting`, the app's `canStopStartingRecording`): the audio stops at once and the
    /// meeting is saved and ended when its row lands. After that Stop the controls say Saving (`stopPendingForRow`). `.unavailable`
    /// is a start in flight that is none of those, where `endActiveMeetingIfNeeded` does nothing on purpose
    /// (`WMenuRecording.stopWaitsForMeeting`).
    static func stopOffer(
        hasActiveMeeting: Bool, isStarting: Bool, audioLive: Bool, canStopStarting: Bool = false, stopPendingForRow: Bool = false
    ) -> WStopOffer {
        WMenuRecordingRules.stopOffer(
            hasActiveMeeting: hasActiveMeeting, isStarting: isStarting, audioLive: audioLive, canStopStarting: canStopStarting,
            stopPendingForRow: stopPendingForRow)
    }

    /// Whether Stop works now: `stopOffer` is `.stop`.
    static func canStop(hasActiveMeeting: Bool, isStarting: Bool, audioLive: Bool, canStopStarting: Bool = false) -> Bool {
        stopOffer(hasActiveMeeting: hasActiveMeeting, isStarting: isStarting, audioLive: audioLive, canStopStarting: canStopStarting) == .stop
    }

    /// The time on the row: nothing, not 0:00, when no start time is known.
    static func elapsed(startedAt: Date?, hasPause: Bool, seconds: Int) -> Int? { startedAt == nil && !hasPause ? nil : seconds }

    /// The tallest the menu may be: the shortest visible screen height, less room for the menu bar's shadow and a margin.
    static func maxHeight(visibleScreenHeights: [CGFloat]) -> CGFloat? {
        guard let shortest = visibleScreenHeights.min() else { return nil }
        return max(320, shortest - 32)
    }

    // MARK: Needs attention

    /// What is missing and what stops working without it. `granted` is a permission's current state.
    struct Permissions: Equatable {
        var screenRecording: Bool
        var microphone: Bool
        var accessibility: Bool
    }

    /// What the menu says for each missing permission: what stops working, then what to do about it.
    static func permissionNeeds(_ permissions: Permissions) -> [WMenuNeed] {
        var out: [WMenuNeed] = []
        if !permissions.screenRecording {
            out.append(WMenuNeed(
                id: "screen-recording", kind: .permission,
                title: String(localized: "System audio can't be recorded.", comment: "Line in the menu bar menu under Needs attention: the Screen & System Audio Recording permission is off, so the other people in a call are not recorded."),
                detail: String(localized: "Allow Screen & System Audio Recording in System Settings.", comment: "Line under a permission problem in the menu bar menu. Screen & System Audio Recording is the macOS name of the permission.")))
        }
        if !permissions.microphone {
            out.append(WMenuNeed(
                id: "microphone", kind: .permission,
                title: String(localized: "Your microphone can't be recorded.", comment: "Line in the menu bar menu under Needs attention: the Microphone permission is off, so the person using this Mac is not recorded."),
                detail: String(localized: "Allow Microphone in System Settings.", comment: "Line under a permission problem in the menu bar menu. Microphone is the macOS name of the permission.")))
        }
        if !permissions.accessibility {
            out.append(WMenuNeed(
                id: "accessibility", kind: .permission,
                title: String(localized: "Whimbrel can't read text on your screen.", comment: "Line in the menu bar menu under Needs attention: the Accessibility permission is off, so screen text is not saved to memory and Ask about this window cannot read a window."),
                detail: String(localized: "Allow Accessibility in System Settings.", comment: "Line under a permission problem in the menu bar menu. Accessibility is the macOS name of the permission.")))
        }
        return out
    }

    /// Why memory is on but nothing is being saved from the screen, when no other row already says so: a missing Accessibility
    /// permission has its own row, and so does a closed library. This covers the rest (capture did not start).
    static func memoryNeed(memory: WMemoryState, accessibilityGranted: Bool, noStoreShown: Bool) -> WMenuNeed? {
        guard memory == .onNotCapturing, accessibilityGranted, !noStoreShown else { return nil }
        return WMenuNeed(
            id: "memory-not-capturing", kind: .memory,
            title: String(localized: "Whimbrel isn\u{2019}t saving text from your screen.", comment: "Line in the menu bar menu under Needs attention: memory is on but nothing is being saved from the screen, and no permission explains it."),
            detail: String(localized: "Turn memory off and on again.", comment: "Line under a memory problem in the menu bar menu: what to try."))
    }

    /// A reason the audio capture gives while a meeting records (a device went away, a stream stopped). The reason is the
    /// capture code's own text, shown as it is.
    static func audioNeeds(recording: Bool, systemAudioReason: String?, micReason: String?) -> [WMenuNeed] {
        guard recording else { return [] }
        var out: [WMenuNeed] = []
        if let systemAudioReason {
            out.append(WMenuNeed(
                id: "system-audio-reason", kind: .audio,
                title: String(localized: "System audio isn't working right.", comment: "Line in the menu bar menu under Needs attention while a meeting records. The reason follows on the next line."),
                detail: systemAudioReason))
        }
        if let micReason {
            out.append(WMenuNeed(
                id: "mic-reason", kind: .audio,
                title: String(localized: "Your microphone isn't working right.", comment: "Line in the menu bar menu under Needs attention while a meeting records. The reason follows on the next line."),
                detail: micReason))
        }
        return out
    }

    /// Sync that has gone quiet. The Sync now button is drawn by the app (`MenuSyncAction`), which keeps its progress and result.
    static func syncNeed(lastSuccess: Date?, stale: Bool, retryPossible: Bool, locale: Locale = .current, timeZone: TimeZone = .current) -> WMenuNeed? {
        guard stale, let lastSuccess else { return nil }
        let when = lastSuccess.formatted(Date.FormatStyle(date: .abbreviated, time: .shortened, locale: locale, timeZone: timeZone))
        // The title already says it is not working, so the line says only when it last did.
        let detail = retryPossible
            ? String(localized: "Last worked \(when). Whimbrel keeps trying.", comment: "Line under Sync is not working in the menu bar menu. The placeholder is the date and time sync last worked, for example Nov 20, 4:10 PM.")
            : String(localized: "Last worked \(when).", comment: "Line under Sync is not working in the menu bar menu. The placeholder is the date and time sync last worked, for example Nov 20, 4:10 PM.")
        return WMenuNeed(id: "sync", kind: .sync, title: String(localized: "Sync is not working.", comment: "Line in the menu bar menu under Needs attention when content sync has stopped. The line under it says since when, and Sync now is beside it."), detail: detail)
    }

    /// The needs in the order the menu shows them: the notices the old menu carried above everything, then what stops
    /// working, then sync.
    static func needs(
        noStore: WMenuNeed?, libraryMissing: WMenuNeed?, memory: WMenuNeed? = nil, permissions: [WMenuNeed], audio: [WMenuNeed], sync: WMenuNeed?
    ) -> [WMenuNeed] {
        [noStore, libraryMissing, memory].compactMap { $0 } + permissions + audio + [sync].compactMap { $0 }
    }

    static func noStoreNeed(openSettings: @escaping () -> Void) -> WMenuNeed {
        WMenuNeed(id: "no-store", kind: .noStore, title: LibraryCopyRedesign.map(EncryptionCopy.noStoreNotice), actions: [WMenuAction(title: EncryptionCopy.openSettingsAction, run: openSettings)])
    }

    static func libraryMissingNeed(openHelp: @escaping () -> Void, dismiss: @escaping () -> Void) -> WMenuNeed {
        WMenuNeed(
            id: "library-missing", kind: .library, title: EncryptionCopy.libraryMissingNotice,
            actions: [WMenuAction(title: EncryptionCopy.libraryBackupHelpLink, isLink: true, run: openHelp), WMenuAction(title: EncryptionCopy.dismiss, run: dismiss)])
    }

    // MARK: Next meeting

    static func next(title: String, start: Date, topic: String?, hasBrief: Bool, locale: Locale = .current, timeZone: TimeZone = .current) -> WMenuNext {
        WMenuNext(title: title, time: start.formatted(Date.FormatStyle(date: .omitted, time: .shortened, locale: locale, timeZone: timeZone)), topic: topic, hasBrief: hasBrief)
    }

    // MARK: The launch menu

    static func launch(waitingForUser: Bool) -> WLaunchMenuModel {
        WLaunchMenuModel(
            message: waitingForUser ? EncryptionCopy.needsAttention : EncryptionCopy.starting, waitingForUser: waitingForUser,
            showTitle: EncryptionCopy.showWhimbrel, quitTitle: EncryptionCopy.quitAction)
    }
}

/// "Sync now" in the menu: the same model and the same rules as `SyncNowControl` (Settings, Devices and the sidebar), in
/// the redesign's button. Its progress and its result stay beside it.
struct MenuSyncAction: View {
    @ObservedObject var sync: ContentSyncCoordinator
    @StateObject private var model: SyncNowModel

    init(sync: ContentSyncCoordinator) {
        self.sync = sync
        _model = StateObject(wrappedValue: SyncNowModel(
            canSync: { [weak sync] in sync?.canSyncNow ?? false },
            run: { [weak sync] in await sync?.syncNowManually() ?? .unavailable }
        ))
    }

    var body: some View {
        let state = SyncNowModel.buttonState(status: sync.syncStatus, roundRunning: sync.roundInFlight, phase: model.phase)
        HStack(spacing: 8) {
            WButton(state == .syncing ? String(localized: "Syncing…", comment: "Shown in place of the Sync now button while its round runs.") : String(localized: "Sync now", comment: "Button that runs one content sync round now (Settings, Devices, and the sidebar's sync warning)."), kind: .secondary) {
                Task { await model.start() }
            }
            .disabled(state != .enabled)
            if let message = model.message {
                Text(message).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary).lineLimit(2)
            }
        }
        // The "Synced" line goes away by itself; an error stays until the next try.
        .task(id: model.phase) {
            guard model.phase == .synced else { return }
            try? await Task.sleep(nanoseconds: 5 * 1_000_000_000)
            if !Task.isCancelled { model.clearOutcome() }
        }
    }
}

/// The menu bar menu with the redesign switch on. It reads what the old menu read and calls what the old menu called.
struct MenuBarRedesignView: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject var coordinator: LaunchCoordinator<CoreMemoryStore>
    @ObservedObject var sync: ContentSyncCoordinator
    @Environment(\.openSettings) private var openSettings
    @Environment(\.openWindow) private var openWindow
    @Environment(\.recordingControls) private var controls
    @State private var nextTopic: TopicRef?
    /// The next meeting's brief key, when it has a readable brief.
    @State private var nextPrepKey: String?
    /// When this menu window appeared, to tell which app the user was in before it.
    @State private var openedAt = Date()
    /// The minute the menu opened in, so the one-minute tick lands on minute boundaries.
    @State private var tickAnchor = Date(timeIntervalSince1970: (Date().timeIntervalSince1970 / 60).rounded(.down) * 60)

    /// The next calendar event that has not ended (the list looks back 15 minutes).
    private var next: QualifyingMeeting? {
        let now = Date()
        return appState.allUpcomingMeetings.filter { $0.endDate > now }.min { $0.startDate < $1.startDate }
    }

    /// While memory is on, not capturing and inside its hold, the moment the hold ends: the menu redraws then, so "Memory on"
    /// becomes "Memory on, but not capturing" at once rather than at the next minute tick.
    private var memoryHoldEnd: Date? {
        guard appState.activityCaptureEnabled else { return nil }
        return WMemoryHold.holdEnds(capturing: appState.screenText.isRunning, since: appState.memoryOnSince, now: Date())
    }

    var body: some View {
        // One timeline, so one view identity: the recorded time moves each second while a recording is held, and otherwise the
        // clock is read each minute (a "Memory paused until 3:10 PM" and a stale sync warning do not go stale while the menu stays
        // open). A recording that starts or stops while the menu is open changes the interval only; two timelines would rebuild
        // the menu and run its `onAppear`, which puts focus back on the first control.
        SwiftUI.TimelineView(WRedrawSchedule(anchor: tickAnchor, interval: appState.isRecordingNow ? 1 : 60, extra: memoryHoldEnd)) { context in menu(now: context.date) }
        .redesignThemed()
        .onAppear { openedAt = Date() }
        // Also rerun when a brief may have landed: this Mac's prep tick, or a sync.
        .task(id: lookupKey) {
            guard let next, let store = appState.coreStore else { nextTopic = nil; nextPrepKey = nil; return }
            let (id, title) = (next.id, next.title)
            let key = TodayLogic.prepKey(eventId: next.id, start: next.startDate)
            let (topic, prepKey) = await withCheckedContinuation { done in
                CoreMemoryStore.accessQueue.async {
                    done.resume(returning: (store.topicForEvent(eventId: id, title: title), store.meetingPrep(key: key)?.brief != nil ? key : nil))
                }
            }
            nextTopic = topic
            nextPrepKey = prepKey
        }
    }

    private var lookupKey: String {
        let nextKey = next.map { TodayLogic.prepKey(eventId: $0.id, start: $0.startDate) } ?? "none"
        return "\(nextKey)|\(appState.routineScheduler.recentRuns.first?.id ?? 0)|\(appState.contentSyncCoordinator.lastSyncedAt?.timeIntervalSince1970 ?? 0)"
    }

    private func menu(now: Date) -> some View {
        let built = model(now: now)
        return WMenuView(
            model: built, actions: actions,
            maxHeight: MenuBarAdapter.maxHeight(visibleScreenHeights: NSScreen.screens.map { $0.visibleFrame.height }),
            memoryGate: { AnyView($0.recordingControl()) },
            startGate: { AnyView($0.recordingControl()) },
            accessory: { id in
                if id == "sync" { MenuSyncAction(sync: sync) }
            },
            extras: {
                #if DEBUG
                Button("Dump Zoom window (AX)") { AXTreeDump.dumpZoomWindows() }
                Button("Record Zoom (60 s)") { AXTreeDump.recordZoomWindows() }
                #endif
            })
    }

    private func model(now: Date) -> WMenuModel {
        let pause = appState.recordingPause
        let hasMeeting = appState.activeMeetingId != nil
        let micLive = appState.audio.micActive, systemLive = appState.audio.systemAudioActive
        let state = LiveMeetingAdapter.state(
            isProcessing: false, stage: nil, isStarting: appState.isStartingMeetingNow, pause: pause,
            resuming: appState.isResumingRecording, problem: appState.recordingProblem)
        // The same rule the old menu's End Meeting follows (`canStopStartingRecording`, `isStopPendingForRow`) and the main window's bar.
        let stopOffer = MenuBarAdapter.stopOffer(
            hasActiveMeeting: hasMeeting, isStarting: appState.isStartingMeetingNow, audioLive: micLive || systemLive,
            canStopStarting: appState.canStopStartingRecording, stopPendingForRow: appState.isStopPendingForRow)
        let recording = MenuBarAdapter.recording(
            holdsRecording: appState.isRecordingNow, state: state,
            elapsedSeconds: MenuBarAdapter.elapsed(
                startedAt: appState.activeMeetingStartedAt, hasPause: pause != nil,
                seconds: LiveMeetingAdapter.recordedSeconds(now: now, startedAt: appState.activeMeetingStartedAt, pause: pause)),
            title: appState.activeMeetingTitle, canPause: appState.canPauseRecording, paused: appState.recordingPaused,
            microphoneLive: micLive, systemAudioLive: systemLive,
            audioReason: appState.audio.degradedReason ?? appState.audio.micDegradedReason,
            canStop: stopOffer == .stop, isSaving: stopOffer == .saving,
            startedAt: WMenuRecordingRules.startAnchor(meetingStartedAt: appState.activeMeetingStartedAt, audioStartedAt: appState.audio.recordingStartedAt),
            now: now)
        let permissions = appState.permissions
        let memory = HomeRedesignAdapter.memoryState(
            enabled: appState.activityCaptureEnabled, resumeAt: appState.captureResumeAt, now: now, capturing: appState.screenText.isRunning, since: appState.memoryOnSince)
        let noStoreShown = appState.recordingGate.showsNotice
        let needs = MenuBarAdapter.needs(
            noStore: noStoreShown ? MenuBarAdapter.noStoreNeed(openSettings: openCaptureSettings) : nil,
            libraryMissing: coordinator.notes.contains(.libraryMissing)
                ? MenuBarAdapter.libraryMissingNeed(
                    openHelp: { NSWorkspace.shared.open(EncryptionCopy.libraryBackupHelpURL) },
                    dismiss: { coordinator.dismissLibraryMissingNotice() })
                : nil,
            memory: MenuBarAdapter.memoryNeed(memory: memory, accessibilityGranted: permissions.accessibilityGranted, noStoreShown: noStoreShown),
            permissions: MenuBarAdapter.permissionNeeds(MenuBarAdapter.Permissions(
                screenRecording: permissions.screenRecordingGranted, microphone: permissions.microphoneGranted, accessibility: permissions.accessibilityGranted)),
            audio: MenuBarAdapter.audioNeeds(
                recording: appState.isRecordingNow, systemAudioReason: appState.audio.degradedReason, micReason: appState.audio.micDegradedReason),
            sync: MenuBarAdapter.syncNeed(lastSuccess: sync.lastSuccess, stale: sync.isStale(now: now), retryPossible: sync.retryPossible))
        // Controls that are drawn but off: arrow keys and the first focus skip them.
        var disabled: Set<String> = []
        if !controls.enabled { disabled.formUnion([WMenuFocusID.memory, WMenuFocusID.start]) }
        if !sync.canSyncNow || sync.roundInFlight { disabled.insert(WMenuFocusID.need("sync", 0)) }
        return WMenuModel(
            recording: recording,
            memory: memory,
            needs: needs,
            next: next.map { MenuBarAdapter.next(title: $0.title, start: $0.startDate, topic: nextTopic?.name, hasBrief: nextPrepKey != nil) },
            startCaption: controls.caption,
            workingOn: appState.processingStage,
            disabled: disabled)
    }

    private func openCaptureSettings() {
        appState.pendingSettingsSection = .capture
        NSApp.activate(ignoringOtherApps: true)
        openSettings()
    }

    private var actions: WMenuActions {
        WMenuActions(
            pauseRecording: { Task { @MainActor in appState.pauseRecording() } },
            resumeRecording: { Task { @MainActor in appState.resumeRecording() } },
            stopRecording: { Task { @MainActor in appState.endActiveMeetingIfNeeded() } },
            showMeetingWindow: { Task { @MainActor in appState.meetingBar.showFromMenu() } },
            pauseMemory: { appState.pauseCapture($0) },
            resumeMemory: { appState.resumeCaptureNow() },
            // Deferred, as the old menu's toggle was: see TimelineView.swift's identical toggle for the reentrancy risk.
            toggleMemory: { Task { @MainActor in appState.toggleActivityCapture() } },
            whatIsCaptured: openCaptureSettings,
            openBrief: { if let nextPrepKey { appState.showPrepBrief(key: nextPrepKey) } },
            openTopic: { if let nextTopic { appState.openTopic(nextTopic.topicId) } },
            startMeeting: { Task { @MainActor in appState.startManualMeeting() } },
            askAboutWindow: { appState.openHotkeyPanelFromMenu(menuOpenedAt: openedAt) },
            openWhimbrel: { openWindow(id: "timeline") },
            openSettings: { openSettings() },
            quit: { NSApplication.shared.terminate(nil) })
    }
}

/// The menu before the library opens, in the redesign. Same behaviour as `LaunchMenuBarView`: it opens the window when
/// the launch waits for the person, and offers Show Whimbrel and Quit.
struct LaunchMenuRedesignView: View {
    @Environment(\.openWindow) private var openWindow
    @ObservedObject var presenter: LaunchUIPresenter

    var body: some View {
        WLaunchMenuView(
            model: MenuBarAdapter.launch(waitingForUser: presenter.waitingForUser),
            onShow: {
                NSApp.activate(ignoringOtherApps: true)
                openWindow(id: "timeline")
            },
            // Through `AppTerminationGuard`: asks first during the setup when needed.
            onQuit: { NSApp.terminate(nil) })
        .redesignThemed()
        .onAppear {
            guard presenter.waitingForUser else { return }
            NSApp.activate(ignoringOtherApps: true)
            openWindow(id: "timeline")
        }
    }
}

/// A periodic timeline schedule that can also fire once at `extra` (the end of the memory hold). It is one type whatever its
/// values, so changing the interval or the extra date keeps the same `TimelineView` identity and focus does not move.
struct WRedrawSchedule: TimelineSchedule {
    var anchor: Date
    var interval: TimeInterval
    var extra: Date?

    func entries(from start: Date, mode: TimelineScheduleMode) -> Entries {
        Entries(schedule: self, cursor: start, first: true)
    }

    struct Entries: Sequence, IteratorProtocol {
        let schedule: WRedrawSchedule
        var cursor: Date
        var first: Bool

        mutating func next() -> Date? {
            if first { first = false; return cursor }
            cursor = WRedrawClock.next(after: cursor, anchor: schedule.anchor, interval: schedule.interval, extra: schedule.extra)
            return cursor
        }
    }
}
