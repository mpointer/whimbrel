import WhimbrelUI
import XCTest
@testable import Whimbrel

/// The menu bar menu's adapter (`UI/Redesign/MenuBarRedesign.swift`): what is shown for each state of the app, in which
/// order, and which controls are offered. Nothing here constructs an `AppState`, reads defaults, the store or audio.
final class MenuBarRedesignTests: XCTestCase {
    private let utc = TimeZone(identifier: "UTC")!
    private let en = Locale(identifier: "en_US")
    /// 2027-01-15 08:00:00 UTC.
    private let at = Date(timeIntervalSince1970: 1_800_000_000)

    // MARK: Recording

    func testPauseIsOfferedWhenItCanRunAndResumeWhilePaused() {
        XCTAssertTrue(MenuBarAdapter.canTogglePause(canPause: true, paused: false))
        XCTAssertTrue(MenuBarAdapter.canTogglePause(canPause: false, paused: true))
        XCTAssertFalse(MenuBarAdapter.canTogglePause(canPause: false, paused: false))
    }

    private func recording(
        holds: Bool = true, state: WLiveState = .recording, elapsed: Int? = 848, title: String? = "Pricing review", canPause: Bool = true,
        paused: Bool = false, mic: Bool = true, system: Bool = true, reason: String? = nil, canStop: Bool = true, isSaving: Bool = false
    ) -> WMenuRecording? {
        MenuBarAdapter.recording(
            holdsRecording: holds, state: state, elapsedSeconds: elapsed, title: title, canPause: canPause, paused: paused,
            microphoneLive: mic, systemAudioLive: system, audioReason: reason, canStop: canStop, isSaving: isSaving)
    }

    func testNoRecordingSectionWhenNothingIsHeld() {
        XCTAssertNil(recording(holds: false))
    }

    func testTheRowShowsWheneverAudioIsCapturedOrAMeetingStartsNotOnlyWithAMeetingRow() throws {
        // `holdsRecording` is `RecordingPauseRules.holdsRecording`: a meeting, a start, either stream or a pause.
        for (meeting, starting, mic, system, paused) in [(true, false, false, false, false), (false, true, false, false, false), (false, false, true, false, false),
                                                         (false, false, false, true, false), (false, false, false, false, true)] {
            XCTAssertTrue(RecordingPauseRules.holdsRecording(hasActiveMeeting: meeting, isStarting: starting, micActive: mic, systemAudioActive: system, paused: paused))
        }
        XCTAssertFalse(RecordingPauseRules.holdsRecording(hasActiveMeeting: false, isStarting: false, micActive: false, systemAudioActive: false, paused: false))
        XCTAssertNotNil(recording(elapsed: nil, title: nil))
    }

    func testTheRecordingSectionCarriesStateTimeAndTitle() throws {
        let r = try XCTUnwrap(recording(state: .paused, canPause: false, paused: true))
        XCTAssertEqual(r.state, .paused)
        XCTAssertEqual(r.elapsedSeconds, 848)
        XCTAssertEqual(r.title, "Pricing review")
        XCTAssertTrue(r.canTogglePause)
        XCTAssertTrue(r.offersResume)
    }

    func testAStartingRecordingWaitsAndItsStopOnlyRunsWhenThereIsSomethingToStop() throws {
        let r = try XCTUnwrap(recording(state: .starting, elapsed: nil, title: nil, canPause: false, mic: false, system: false, canStop: false))
        XCTAssertEqual(r.state, .starting)
        XCTAssertFalse(r.canTogglePause)
        XCTAssertFalse(r.canStop)
        XCTAssertTrue(MenuBarAdapter.canStop(hasActiveMeeting: true, isStarting: true, audioLive: false))
        XCTAssertTrue(MenuBarAdapter.canStop(hasActiveMeeting: false, isStarting: false, audioLive: true))
        // A start still in flight with no meeting row: `endActiveMeetingIfNeeded` does nothing, so Stop is off.
        XCTAssertFalse(MenuBarAdapter.canStop(hasActiveMeeting: false, isStarting: true, audioLive: true))
        XCTAssertFalse(MenuBarAdapter.canStop(hasActiveMeeting: false, isStarting: false, audioLive: false))
    }

    /// #992: a detected or calendar start turns its audio on before its meeting row is saved. With the flag on, the menu and the
    /// main window's bar offer the Stop the old menu's End Meeting offers (`AppState.canStopStartingRecording`), and say Saving after
    /// it (`isStopPendingForRow`). Every row is a plain value here: no AppState, no clock, no defaults.
    func testStopOfferForEveryStateOfAStart() {
        func offer(meeting: Bool = false, starting: Bool, audio: Bool, canStopStarting: Bool, pending: Bool) -> WStopOffer {
            MenuBarAdapter.stopOffer(hasActiveMeeting: meeting, isStarting: starting, audioLive: audio, canStopStarting: canStopStarting, stopPendingForRow: pending)
        }
        // The start's audio is on and no row yet: Stop works (the state `AppState.canStopStartingRecording` reports).
        XCTAssertEqual(offer(starting: true, audio: true, canStopStarting: true, pending: false), .stop)
        // Stop asked for, row not saved: Saving, and no Stop, with the audio already off or still settling.
        XCTAssertEqual(offer(starting: true, audio: false, canStopStarting: false, pending: true), .saving)
        XCTAssertEqual(offer(starting: true, audio: true, canStopStarting: false, pending: true), .saving)
        // Any other start: Stop is off, and the sentence says why when audio is live.
        XCTAssertEqual(offer(starting: true, audio: true, canStopStarting: false, pending: false), .unavailable)
        XCTAssertEqual(offer(starting: true, audio: false, canStopStarting: false, pending: false), .unavailable)
        // A meeting is open, or audio runs with no row after a failed insert: Stop.
        XCTAssertEqual(offer(meeting: true, starting: true, audio: false, canStopStarting: false, pending: false), .stop)
        XCTAssertEqual(offer(starting: false, audio: true, canStopStarting: false, pending: false), .stop)
        XCTAssertEqual(offer(starting: false, audio: false, canStopStarting: false, pending: false), .unavailable)
        // `canStop` is the same rule, so a menu with Stop on cannot disagree with the bar.
        XCTAssertTrue(MenuBarAdapter.canStop(hasActiveMeeting: false, isStarting: true, audioLive: true, canStopStarting: true))
        XCTAssertFalse(MenuBarAdapter.canStop(hasActiveMeeting: false, isStarting: true, audioLive: true, canStopStarting: false))
    }

    func testTheMenuRowCarriesStopOrSavingAndTheSentenceOnlyWhereStopCannotWork() throws {
        let starting = { (offer: WStopOffer) in
            self.recording(state: .starting, elapsed: nil, title: "Design sync", canPause: false, mic: true, system: true, canStop: offer == .stop, isSaving: offer == .saving)
        }
        let stop = try XCTUnwrap(starting(.stop))
        XCTAssertTrue(stop.canStop && !stop.isSaving && !stop.stopWaitsForMeeting && stop.offersStop)
        let saving = try XCTUnwrap(starting(.saving))
        XCTAssertTrue(saving.isSaving && !saving.canStop && !saving.stopWaitsForMeeting)
        let waiting = try XCTUnwrap(starting(.unavailable))
        XCTAssertTrue(waiting.stopWaitsForMeeting && !waiting.canStop && !waiting.isSaving)
    }

    func testAProblemKeepsItsTimeItsTitleAndItsReasonAndOffersStopOnly() throws {
        // `AppState.resumeRecording` ignores anything but a pause, so Resume would do nothing here; Stop is the way out.
        let r = try XCTUnwrap(recording(state: .problem("The microphone stopped."), mic: false, system: true))
        XCTAssertEqual(r.state, .problem("The microphone stopped."))
        XCTAssertEqual(r.elapsedSeconds, 848)
        XCTAssertEqual(r.title, "Pricing review")
        XCTAssertFalse(r.showsPauseButton)
        XCTAssertTrue(r.offersStop)
        XCTAssertEqual(WMenuText.stateWord(r.state, strings: .english), "Recording problem")
        XCTAssertEqual(r.streams, WMenuStreams(microphone: false, systemAudio: true))
    }

    func testARunningRecordingWithNeitherStreamLiveIsAProblemNotSilence() throws {
        let r = try XCTUnwrap(recording(state: .recording, mic: false, system: false, reason: "System audio stopped."))
        XCTAssertEqual(r.state, .problem("System audio stopped."))
        let noReason = try XCTUnwrap(recording(state: .recording, mic: false, system: false))
        XCTAssertEqual(noReason.state, .problem(""))
        // The streams line says it; the generic line would only repeat it.
        XCTAssertNil(WMenuText.problemLine(noReason, strings: .english))
        XCTAssertEqual(WMenuText.streamsLine(noReason, strings: .english), "No audio is being captured.")
    }

    func testAPausedOrStartingRecordingIsNotJudgedByTheStreamFlags() throws {
        // A pause leaves the flags set while nothing is captured; a start has none yet.
        XCTAssertEqual(try XCTUnwrap(recording(state: .paused, mic: false, system: false)).state, .paused)
        XCTAssertEqual(try XCTUnwrap(recording(state: .starting, mic: false, system: false)).state, .starting)
        XCTAssertEqual(try XCTUnwrap(recording(state: .resuming, mic: false, system: false)).state, .resuming)
    }

    func testAJustStartedRecordingSaysItIsStartingAndOnlyLaterIsSilenceAProblem() throws {
        func state(after seconds: TimeInterval, started: Date? = nil) throws -> WRecordingState {
            let r = MenuBarAdapter.recording(
                holdsRecording: true, state: .recording, elapsedSeconds: 0, title: "Meeting", canPause: true, paused: false,
                microphoneLive: false, systemAudioLive: false, startedAt: started ?? at, now: at.addingTimeInterval(seconds))
            return try XCTUnwrap(r).state
        }
        XCTAssertEqual(try state(after: 0), .starting)
        XCTAssertEqual(try state(after: 4), .starting)
        XCTAssertEqual(try state(after: 10), .problem(""))
        // The meeting's start and the audio's start: the later one belongs to this start.
        XCTAssertEqual(WMenuRecordingRules.startAnchor(meetingStartedAt: at, audioStartedAt: at.addingTimeInterval(-3600)), at)
        // Paused is never judged, early or late.
        let paused = try XCTUnwrap(MenuBarAdapter.recording(
            holdsRecording: true, state: .paused, elapsedSeconds: 0, title: nil, canPause: false, paused: true,
            microphoneLive: false, systemAudioLive: false, startedAt: at, now: at.addingTimeInterval(60)))
        XCTAssertEqual(paused.state, .paused)
    }

    func testMemoryThatIsOnAndNotSavingIsCalledAProblemOnlyAfterAFewSeconds() {
        XCTAssertEqual(HomeRedesignAdapter.memoryState(enabled: true, resumeAt: nil, now: at.addingTimeInterval(2), capturing: false, since: at), .on)
        XCTAssertEqual(HomeRedesignAdapter.memoryState(enabled: true, resumeAt: nil, now: at.addingTimeInterval(8), capturing: false, since: at), .onNotCapturing)
        XCTAssertEqual(HomeRedesignAdapter.memoryState(enabled: true, resumeAt: nil, now: at, capturing: false), .onNotCapturing)
    }

    func testTheStreamsAreTheLiveOnes() throws {
        XCTAssertEqual(try XCTUnwrap(recording(mic: true, system: false)).streams, WMenuStreams(microphone: true, systemAudio: false))
        XCTAssertEqual(WMenuText.streamsLine(try XCTUnwrap(recording(mic: true, system: false)), strings: .english), "Microphone only")
        XCTAssertEqual(WMenuText.streamsLine(try XCTUnwrap(recording(mic: false, system: true)), strings: .english), "System audio only")
        XCTAssertEqual(WMenuText.streamsLine(try XCTUnwrap(recording()), strings: .english), "Microphone and system audio")
    }

    func testNoTimeIsSaidWhenNoStartTimeIsKnown() {
        XCTAssertNil(MenuBarAdapter.elapsed(startedAt: nil, hasPause: false, seconds: 0))
        XCTAssertEqual(MenuBarAdapter.elapsed(startedAt: nil, hasPause: true, seconds: 12), 12)
        XCTAssertEqual(MenuBarAdapter.elapsed(startedAt: at, hasPause: false, seconds: 0), 0)
    }

    func testTheMenuIsNoTallerThanTheShortestScreen() {
        XCTAssertEqual(MenuBarAdapter.maxHeight(visibleScreenHeights: [900, 1200]), 868)
        XCTAssertEqual(MenuBarAdapter.maxHeight(visibleScreenHeights: [200]), 320)
        XCTAssertNil(MenuBarAdapter.maxHeight(visibleScreenHeights: []))
    }

    // MARK: Memory that is on but not capturing

    func testMemoryThatIsOnWithoutScreenTextRunningIsNotCalledMemoryOn() {
        XCTAssertEqual(HomeRedesignAdapter.memoryState(enabled: true, resumeAt: nil, now: at, capturing: true), .on)
        XCTAssertEqual(HomeRedesignAdapter.memoryState(enabled: true, resumeAt: nil, now: at, capturing: false), .onNotCapturing)
        // Off and a timed pause are what they are, whatever the capture is doing.
        XCTAssertEqual(HomeRedesignAdapter.memoryState(enabled: false, resumeAt: nil, now: at, capturing: false), .off)
        // A timed pause is Memory switched off with a time to come back stored (`CapturePauseModel.state`).
        if case .pausedUntil = HomeRedesignAdapter.memoryState(enabled: false, resumeAt: at.addingTimeInterval(900), now: at, capturing: false) {} else { XCTFail("expected a pause") }
        // The default keeps every other caller as it was.
        XCTAssertEqual(HomeRedesignAdapter.memoryState(enabled: true, resumeAt: nil, now: at), .on)
    }

    func testTheShellAndTheMenuReadTheSameRule() throws {
        let shell = try String(contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("Whimbrel/UI/WhimbrelShellView.swift"), encoding: .utf8)
        let menu = try String(contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("Whimbrel/UI/Redesign/MenuBarRedesign.swift"), encoding: .utf8)
        XCTAssertTrue(shell.contains("capturing: appState.screenText.isRunning"))
        XCTAssertTrue(menu.contains("capturing: appState.screenText.isRunning"))
    }

    func testTheMemoryWordsSayNotCapturingAndSayMemoryWhenPaused() {
        XCTAssertEqual(HomeRedesignAdapter.nowLabel(.onNotCapturing), String(localized: "Memory on, but not capturing"))
        XCTAssertNotNil(HomeRedesignAdapter.notice(.onNotCapturing))
        XCTAssertEqual(HomeRedesignAdapter.nowLabel(.pausedUntil("3:10 PM")), String(localized: "Memory paused until \("3:10 PM")"))
        XCTAssertEqual(WhimbrelUIStrings.make().pausedUntil("3:10 PM"), String(localized: "Memory paused until \("3:10 PM")"))
        XCTAssertEqual(WhimbrelUIStrings.make().memoryNotCapturing, String(localized: "Memory on, but not capturing"))
    }

    func testAMissingAccessibilityPermissionIsItsOwnReasonRowAndNothingIsSaidTwice() {
        XCTAssertNil(MenuBarAdapter.memoryNeed(memory: .on, accessibilityGranted: true, noStoreShown: false))
        XCTAssertNil(MenuBarAdapter.memoryNeed(memory: .off, accessibilityGranted: true, noStoreShown: false))
        // The permission has its own row ("Whimbrel can't read text on your screen."), so there is no second one.
        XCTAssertNil(MenuBarAdapter.memoryNeed(memory: .onNotCapturing, accessibilityGranted: false, noStoreShown: false))
        XCTAssertTrue(MenuBarAdapter.permissionNeeds(.init(screenRecording: true, microphone: true, accessibility: false)).map(\.id) == ["accessibility"])
        XCTAssertNil(MenuBarAdapter.memoryNeed(memory: .onNotCapturing, accessibilityGranted: true, noStoreShown: true))
        let other = MenuBarAdapter.memoryNeed(memory: .onNotCapturing, accessibilityGranted: true, noStoreShown: false)
        XCTAssertEqual(other?.id, "memory-not-capturing")
        XCTAssertEqual(other?.kind, .memory)
        XCTAssertNotNil(other?.detail)
    }

    // MARK: Needs attention

    func testNothingIsMissingWhenEverythingIsAllowed() {
        XCTAssertTrue(MenuBarAdapter.permissionNeeds(.init(screenRecording: true, microphone: true, accessibility: true)).isEmpty)
    }

    func testEachMissingPermissionSaysWhatStopsWorking() {
        let all = MenuBarAdapter.permissionNeeds(.init(screenRecording: false, microphone: false, accessibility: false))
        XCTAssertEqual(all.map(\.id), ["screen-recording", "microphone", "accessibility"])
        XCTAssertTrue(all.allSatisfy { $0.kind == .permission && $0.detail?.isEmpty == false && !$0.title.isEmpty })
        XCTAssertEqual(MenuBarAdapter.permissionNeeds(.init(screenRecording: true, microphone: false, accessibility: true)).map(\.id), ["microphone"])
    }

    func testAudioReasonsShowOnlyWhileRecordingAndKeepTheirOwnText() {
        XCTAssertTrue(MenuBarAdapter.audioNeeds(recording: false, systemAudioReason: "a", micReason: "b").isEmpty)
        let needs = MenuBarAdapter.audioNeeds(recording: true, systemAudioReason: "Stream stopped", micReason: nil)
        XCTAssertEqual(needs.map(\.id), ["system-audio-reason"])
        XCTAssertEqual(needs.first?.detail, "Stream stopped")
        XCTAssertEqual(MenuBarAdapter.audioNeeds(recording: true, systemAudioReason: "a", micReason: "b").map(\.id), ["system-audio-reason", "mic-reason"])
    }

    func testSyncIsAWarningOnlyWhenStaleWithALastSuccess() {
        XCTAssertNil(MenuBarAdapter.syncNeed(lastSuccess: at, stale: false, retryPossible: true))
        XCTAssertNil(MenuBarAdapter.syncNeed(lastSuccess: nil, stale: true, retryPossible: true))
    }

    func testSyncSaysSinceWhenAndWhetherItKeepsTrying() throws {
        let keeps = try XCTUnwrap(MenuBarAdapter.syncNeed(lastSuccess: at, stale: true, retryPossible: true, locale: en, timeZone: utc))
        let stops = try XCTUnwrap(MenuBarAdapter.syncNeed(lastSuccess: at, stale: true, retryPossible: false, locale: en, timeZone: utc))
        XCTAssertEqual(keeps.id, "sync")
        XCTAssertEqual(keeps.kind, .sync)
        XCTAssertTrue(keeps.detail?.contains("Jan 15") == true && keeps.detail?.contains("8:00") == true)
        XCTAssertNotEqual(keeps.detail, stops.detail)
        // The title says it is not working; the line says only when it last did, with no repeat of the title.
        XCTAssertEqual(keeps.title, String(localized: "Sync is not working."))
        // The words come from the app's catalog, so the test asks the same lookup instead of assuming English.
        let when = at.formatted(Date.FormatStyle(date: .abbreviated, time: .shortened, locale: en, timeZone: utc))
        XCTAssertEqual(stops.detail, String(localized: "Last worked \(when)."))
        XCTAssertEqual(keeps.detail, String(localized: "Last worked \(when). Whimbrel keeps trying."))
        XCTAssertFalse(stops.detail?.lowercased().contains("hasn") == true)
        XCTAssertFalse(stops.detail?.lowercased().contains("not working") == true)
        // The button is the app's (MenuSyncAction), so the need itself carries no action.
        XCTAssertTrue(keeps.actions.isEmpty)
    }

    func testTheNoticesComeFirstThenPermissionsThenAudioThenSync() {
        func need(_ id: String) -> WMenuNeed { WMenuNeed(id: id, kind: .permission, title: id) }
        let out = MenuBarAdapter.needs(noStore: need("no-store"), libraryMissing: need("library"), permissions: [need("p1"), need("p2")], audio: [need("a")], sync: need("sync"))
        XCTAssertEqual(out.map(\.id), ["no-store", "library", "p1", "p2", "a", "sync"])
        XCTAssertEqual(MenuBarAdapter.needs(noStore: nil, libraryMissing: nil, memory: need("m"), permissions: [need("p")], audio: [], sync: nil).map(\.id), ["m", "p"])
        XCTAssertTrue(MenuBarAdapter.needs(noStore: nil, libraryMissing: nil, permissions: [], audio: [], sync: nil).isEmpty)
    }

    func testTheNoStoreNoticeKeepsItsWordsAndItsButton() {
        var opened = 0
        let need = MenuBarAdapter.noStoreNeed { opened += 1 }
        XCTAssertEqual(need.title, LibraryCopyRedesign.map(EncryptionCopy.noStoreNotice))
        XCTAssertEqual(need.actions.map(\.title), [EncryptionCopy.openSettingsAction])
        need.actions[0].run()
        XCTAssertEqual(opened, 1)
    }

    func testTheLibraryNoticeKeepsItsWordsItsHelpLinkAndItsDismiss() {
        var log: [String] = []
        let need = MenuBarAdapter.libraryMissingNeed(openHelp: { log.append("help") }, dismiss: { log.append("dismiss") })
        XCTAssertEqual(need.title, EncryptionCopy.libraryMissingNotice)
        XCTAssertEqual(need.actions.map(\.title), [EncryptionCopy.libraryBackupHelpLink, EncryptionCopy.dismiss])
        // The help link opens a web page: a screen reader says "link", as it did in the old menu.
        XCTAssertEqual(need.actions.map(\.isLink), [true, false])
        need.actions.forEach { $0.run() }
        XCTAssertEqual(log, ["help", "dismiss"])
    }

    // MARK: Next meeting

    func testTheNextMeetingCarriesTitleTimeTopicAndBrief() {
        let next = MenuBarAdapter.next(title: "Design sync", start: at, topic: "Pricing", hasBrief: true, locale: en, timeZone: utc)
        XCTAssertEqual(next.title, "Design sync")
        XCTAssertTrue(next.time.contains("8:00"))
        XCTAssertEqual(next.topic, "Pricing")
        XCTAssertTrue(next.hasBrief)
        XCTAssertNil(MenuBarAdapter.next(title: "x", start: at, topic: nil, hasBrief: false).topic)
    }

    // MARK: Launch menu

    func testTheLaunchMenuKeepsItsWords() {
        let waiting = MenuBarAdapter.launch(waitingForUser: true)
        XCTAssertEqual(waiting.message, EncryptionCopy.needsAttention)
        XCTAssertTrue(waiting.waitingForUser)
        let starting = MenuBarAdapter.launch(waitingForUser: false)
        XCTAssertEqual(starting.message, EncryptionCopy.starting)
        XCTAssertEqual(starting.showTitle, EncryptionCopy.showWhimbrel)
        XCTAssertEqual(starting.quitTitle, EncryptionCopy.quitAction)
    }

    // MARK: The old status line is gone from the new menu

    func testTheNewMenuDoesNotUseTheDeveloperStatusLine() throws {
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel/UI/Redesign/MenuBarRedesign.swift")
        let text = try String(contentsOf: url, encoding: .utf8)
        XCTAssertFalse(text.contains("statusSummary"))
        XCTAssertFalse(text.contains("inactiveCapabilities"))
    }
}
