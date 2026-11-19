import XCTest
@testable import Whimbrel

/// Running without the memory database (Mike's decision 1B, Task 11b-2): after "Continue
/// without the memory database", no recording starts by any path, background capture and
/// sync stay off, and a persistent notice shows outside Settings. With a store, nothing
/// changes. `AppState` cannot be built in the test host, so the gate is tested directly
/// with injected fakes, and a source check holds every start entry point in `AppState` to it.
@MainActor
final class NoStoreModeTests: XCTestCase {
    func testWithoutAStoreEveryRecordingStartIsRefusedAndTheUserIsTold() {
        var told: [String] = []
        let gate = RecordingStartGate(storeOpen: false) { told.append($0) }
        for source in RecordingStartGate.Source.allCases {
            XCTAssertFalse(gate.allow(source), "\(source)")
        }
        // Every refused start says why (prompt accept, Away Mode, Today, menu bar).
        XCTAssertEqual(told, Array(repeating: EncryptionCopy.recordingBlockedNoStore, count: RecordingStartGate.Source.allCases.count))
        XCTAssertFalse(gate.allowsBackgroundCapture)
        XCTAssertFalse(gate.allowsMeetingDetection)
        XCTAssertFalse(gate.allowsSync)
        XCTAssertTrue(gate.showsNotice)
    }

    func testWithAStoreNothingChanges() {
        var told: [String] = []
        let gate = RecordingStartGate(storeOpen: true) { told.append($0) }
        for source in RecordingStartGate.Source.allCases {
            XCTAssertTrue(gate.allow(source), "\(source)")
        }
        XCTAssertEqual(told, [])
        XCTAssertTrue(gate.allowsBackgroundCapture)
        XCTAssertTrue(gate.allowsMeetingDetection)
        XCTAssertTrue(gate.allowsSync)
        XCTAssertFalse(gate.showsNotice)
    }

    func testEachTriggerMapsToASource() {
        XCTAssertEqual(RecordingStartGate.Source(trigger: .calendar), .calendarOrPrompt)
        XCTAssertEqual(RecordingStartGate.Source(trigger: .detected), .detectedMeeting)
        XCTAssertEqual(RecordingStartGate.Source(trigger: .away), .awayMode)
        XCTAssertEqual(RecordingStartGate.Source(trigger: .manual), .manual)
    }

    func testTheNoticeCopy() {
        XCTAssertEqual(EncryptionCopy.noStoreNotice,
                       "The memory database could not be opened. Recording is off. Restart Whimbrel to try again.")
        XCTAssertEqual(EncryptionCopy.recordingBlockedNoStore,
                       "Whimbrel can't record meetings because the memory database could not be opened.")
        for s in EncryptionCopy.all { XCTAssertFalse(s.contains("Recording is paused"), s) }
        XCTAssertTrue(EncryptionCopy.all.contains(EncryptionCopy.noStoreNotice))
    }

    // MARK: The controls and the in-window message

    func testRecordingControlsAreOffWithACaptionWithoutAStore() {
        let off = RecordingControls(storeOpen: false)
        XCTAssertFalse(off.enabled)
        XCTAssertEqual(off.caption, EncryptionCopy.recordingOffCaption)
        let on = RecordingControls(storeOpen: true)
        XCTAssertTrue(on.enabled)
        XCTAssertNil(on.caption)
    }

    /// A refused start that got past the disabled controls (a prompt, Away Mode, a link)
    /// shows in the window, not only as a notification (none shows while Whimbrel is in front).
    func testARefusalSurfacesInTheWindow() {
        let refusal = RecordingRefusal()
        var notified: [String] = []
        let gate = RecordingStartGate(storeOpen: false) { message in
            refusal.show(message)
            notified.append(message)
        }
        XCTAssertNil(refusal.message)
        XCTAssertFalse(gate.allow(.calendarOrPrompt))
        XCTAssertEqual(refusal.message, EncryptionCopy.recordingBlockedNoStore)
        XCTAssertEqual(notified.count, 1)
        refusal.dismiss()
        XCTAssertNil(refusal.message)
    }

    private func source(_ rel: String) throws -> String {
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel").appendingPathComponent(rel)
        return try String(contentsOf: url, encoding: .utf8)
    }

    /// Every Start control on screen carries `recordingControl()` (disabled, with a caption or
    /// a tooltip, without a store): the menu bar, the timeline header and every Today skin.
    func testEveryStartControlIsARecordingControl() throws {
        let sites = ["UI/MenuBarContentView.swift": 3, "UI/TimelineView+Header.swift": 2, "UI/Today/TodaySlots.swift": 1,
                     "UI/Today/Archetypes/TerminalTodayLayout.swift": 1, "UI/Today/Archetypes/FieldTodayLayout.swift": 3,
                     "UI/Today/Archetypes/GlassTodayLayout.swift": 1, "UI/Today/Archetypes/ReferenceTodayLayout.swift": 2,
                     "UI/Today/Archetypes/AlmanacTodayLayout.swift": 1, "UI/Today/TodayChrome.swift": 2,
                     "UI/Today/TodayTimeline.swift": 1]
        for (file, count) in sites {
            let text = try source(file)
            XCTAssertEqual(text.components(separatedBy: ".recordingControl()").count - 1, count, file)
        }
        // The state reaches every control from the main window's and the menu bar's roots.
        XCTAssertEqual(try source("WhimbrelApp.swift").components(separatedBy: ".environment(\\.recordingControls, appState.recordingControls)").count - 1, 2)
        XCTAssertTrue(try source("WhimbrelApp.swift").contains("appState.recordingRefusal"))
    }

    // MARK: Every entry point in AppState goes through the gate

    private func appState() throws -> String {
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel/AppState.swift")
        return try String(contentsOf: url, encoding: .utf8)
    }

    /// The text of `signature`'s body up to the next member at the same indentation.
    private func body(_ text: String, _ signature: String) throws -> String {
        let start = try XCTUnwrap(text.range(of: signature), signature)
        let rest = text[start.upperBound...]
        let end = rest.range(of: "\n    func ") ?? rest.range(of: "\n    private func ") ?? rest.endIndex..<rest.endIndex
        return String(rest[..<end.lowerBound])
    }

    func testEveryRecordingStartInAppStateAsksTheGateFirst() throws {
        let text = try appState()
        for (signature, call) in [
            ("func startManualMeeting() {", "recordingGate.allow(.manual)"),
            ("func startCapture(for meeting: QualifyingMeeting, trigger: CoreMeetingTrigger = .calendar) {",
             "recordingGate.allow(RecordingStartGate.Source(trigger: trigger))"),
        ] {
            let b = try body(text, signature)
            let gate = try XCTUnwrap(b.range(of: call), "\(signature) does not ask the gate")
            let firstEffect = b.range(of: "isStartingMeeting = true")
            XCTAssertNotNil(firstEffect)
            if let firstEffect { XCTAssertLessThan(gate.lowerBound, firstEffect.lowerBound, "gate first in \(signature)") }
        }
        // Background screen capture (both places that start it).
        XCTAssertTrue(try body(text, "private func resumeActivityCapture(").contains("recordingGate.allowsBackgroundCapture"))
        XCTAssertTrue(text.contains("if self.activityCaptureEnabled, self.recordingGate.allowsBackgroundCapture {"))
        // Meeting detection and calendar prompts, and sync.
        XCTAssertTrue(text.contains("if enabled, self.recordingGate.allowsMeetingDetection {"))
        XCTAssertTrue(text.contains("if recordingGate.allowsMeetingDetection { meetingTrigger.start("))
        XCTAssertTrue(text.contains("if recordingGate.allowsSync { settingsSyncCoordinator.observeSyncState() }"))
        XCTAssertTrue(text.contains("if recordingGate.allowsSync { contentSyncCoordinator.observeSyncState() }"))
        // No other caller starts recording, screen text, detection or sync unguarded
        // (counted on code lines, comments stripped).
        let code = text.split(separator: "\n", omittingEmptySubsequences: false).map { line -> String in
            guard let r = line.range(of: "//") else { return String(line) }
            return String(line[..<r.lowerBound])
        }.joined(separator: "\n")
        func count(_ s: String) -> Int { code.components(separatedBy: s).count - 1 }
        XCTAssertEqual(count("audio.start()"), 2, "startManualMeeting's and startCapture's own audio starts")
        XCTAssertEqual(count("meetingHeuristicDetector.start()"), 1)
        XCTAssertEqual(count("meetingTrigger.start("), 1)
        XCTAssertEqual(count("screenText.start()"), 2)
        XCTAssertEqual(count("observeSyncState()"), 2)
    }

    // MARK: A recording never starts without its meeting row (#850)

    /// Like `body`, but ends at whichever member comes first (`body` prefers a later `func`).
    private func tightBody(_ text: String, _ signature: String) throws -> String {
        let start = try XCTUnwrap(text.range(of: signature), signature)
        let rest = text[start.upperBound...]
        let ends = ["\n    func ", "\n    private func ", "\n    fileprivate func "].compactMap { rest.range(of: $0)?.lowerBound }
        return String(rest[..<(ends.min() ?? rest.endIndex)])
    }

    func testAStartWhoseInsertFailedRefusesWithTheNotSavedMessage() {
        XCTAssertEqual(RecordingStartInsert.outcome(insertedMeetingId: 42), .proceed(meetingId: 42))
        XCTAssertEqual(RecordingStartInsert.outcome(insertedMeetingId: nil), .refuse(message: EncryptionCopy.recordingNotSaved))
        // It goes through the same refusal UI as a start with no store.
        let refusal = RecordingRefusal()
        let gate = RecordingStartGate(storeOpen: true) { refusal.show($0) }
        if case let .refuse(message) = RecordingStartInsert.outcome(insertedMeetingId: nil) { gate.notify(message) }
        XCTAssertEqual(refusal.message, EncryptionCopy.recordingNotSaved)
    }

    func testQuittingWaitsForAStartWhoseInsertIsStillQueued() {
        let starts = PendingRecordingStarts()
        XCTAssertEqual(starts.quitReply(timeout: 20) { XCTFail("nothing pending: no later reply") }, .terminateNow,
                       "nothing pending: quit at once")
        starts.begin()
        XCTAssertEqual(starts.pending, 1)
        let replied = expectation(description: "quit replied")
        XCTAssertEqual(starts.quitReply(timeout: 20) { replied.fulfill() }, .terminateLater, "a queued insert holds the quit")
        DispatchQueue.global().asyncAfter(deadline: .now() + 0.05) { starts.end() }
        wait(for: [replied], timeout: 5)
        XCTAssertEqual(starts.pending, 0)
    }

    /// The continue path finishes its start on the main thread (`continueCapture`). The quit wait
    /// must leave main free for it: the start lands and the quit goes ahead long before the cap,
    /// instead of main hanging for the whole cap (#850 review, I3).
    func testQuitDoesNotBlockAStartWhoseCompletionNeedsTheMainThread() {
        let starts = PendingRecordingStarts()
        starts.begin()
        var landedOnMain = false
        DispatchQueue.main.async {
            landedOnMain = true
            starts.end()
        }
        let started = Date()
        let replied = expectation(description: "quit replied")
        XCTAssertEqual(starts.quitReply(timeout: 20) { replied.fulfill() }, .terminateLater)
        XCTAssertFalse(landedOnMain, "the quit answer came back without running the start")
        wait(for: [replied], timeout: 10)
        XCTAssertTrue(landedOnMain)
        XCTAssertEqual(starts.pending, 0)
        XCTAssertLessThan(Date().timeIntervalSince(started), 10, "not held for the 20 s cap")
    }

    /// A start that never lands holds the quit only until the cap; a second quit meanwhile does
    /// not start a second wait, and the first one still replies exactly once.
    func testAStuckStartHoldsTheQuitOnlyUntilTheCap() {
        let starts = PendingRecordingStarts()
        starts.begin()
        let replied = expectation(description: "quit replied at the cap")
        replied.assertForOverFulfill = true
        XCTAssertEqual(starts.quitReply(timeout: 0.2) { replied.fulfill() }, .terminateLater)
        XCTAssertEqual(starts.quitReply(timeout: 0.2) { XCTFail("a second quit must not reply") }, .terminateCancel)
        wait(for: [replied], timeout: 5)
        // The start lands late: no second reply (the cap already answered).
        starts.end()
        let drained = expectation(description: "main drained")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { drained.fulfill() }
        wait(for: [drained], timeout: 5)
    }

    /// Both start paths refuse when the row is missing, queue their insert behind the pending-start
    /// counter, and quitting waits for it.
    func testBothStartPathsRefuseWithoutARowAndQuitWaitsForThem() throws {
        let text = try appState()
        for signature in ["private func finishStartingManualCapture(", "private func finishStartingCapture("] {
            let b = try tightBody(text, signature)
            XCTAssertTrue(b.contains("RecordingStartInsert.outcome(insertedMeetingId: insertedMeetingId)"), signature)
            XCTAssertTrue(b.contains("refuseStartWithoutRow()"), signature)
        }
        XCTAssertTrue(try tightBody(text, "private func finishStartingCapture(").contains("stopRowlessRecording()"),
                      "audio started ahead of the insert is stopped")
        XCTAssertEqual(text.components(separatedBy: "PendingRecordingStarts.shared.begin()").count - 1, 2)
        // The continue path is counted until its reopen, part insert and fallback insert are done.
        let cont = try tightBody(text, "private func continueCapture(")
        XCTAssertEqual(cont.components(separatedBy: "PendingRecordingStarts.shared.end()").count - 1, 3)
        // The start hops to `continueCapture` still counted (no `end()` before the hop), and releases
        // the count itself only when the app state is gone.
        let start = try tightBody(text, "func startCapture(for meeting: QualifyingMeeting")
        let hop = "DispatchQueue.main.async { [weak self] in\n                    guard let self else {\n                        PendingRecordingStarts.shared.end()\n                        return\n                    }\n                    self.continueCapture("
        XCTAssertTrue(start.contains(hop), "a nil self releases the pending start; otherwise continueCapture ends it")
        let beforeHop = try XCTUnwrap(start.range(of: hop)).lowerBound
        let tail = start[..<beforeHop].suffix(400)
        XCTAssertFalse(tail.contains("PendingRecordingStarts.shared.end()"), "not ended before the hop to continueCapture")
        XCTAssertTrue(tail.contains("if let previous {"), "the hop is the continue branch")
        for signature in ["func startManualMeeting(", "func startCapture(for meeting: QualifyingMeeting"] {
            XCTAssertTrue(try tightBody(text, signature).contains("PendingRecordingStarts.shared.end()"), signature)
        }
        let guardSource = try String(contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel/Support/AppLaunchLive.swift"), encoding: .utf8)
        XCTAssertTrue(guardSource.contains("PendingRecordingStarts.shared.quitReply(timeout: PendingRecordingStarts.quitWaitSeconds)"))
        XCTAssertTrue(guardSource.contains("NSApp.reply(toApplicationShouldTerminate: true)"), "the later quit is answered")
    }

    /// Background maintenance is on the background lane, screens stay on `accessQueue`.
    func testBackgroundJobsRunOnTheBackgroundLane() throws {
        let text = try appState()
        for step in ["scheduleJobsBatchStep", "scheduleEpisodeSegmentStep", "scheduleScreenTextIngestStep", "scheduleTranscriptBackfillStep"] {
            let b = try tightBody(text, "private func \(step)(")
            XCTAssertTrue(b.contains("backgroundLane"), step)
            XCTAssertFalse(b.contains("CoreMemoryStore.accessQueue"), "\(step) must not hold the screens' queue")
        }
        XCTAssertTrue(text.contains("backgroundStore = coreStore.flatMap"))
        XCTAssertTrue(text.contains("backgroundStore.map { ($0, CoreMemoryStore.backgroundQueue) }"), "the push-cursor reset shares the sync lane")
    }

    /// Background model jobs have their own lane, so a typed question (slow lane) never queues behind them,
    /// and each Ollama-capable one checks the user gate before it starts a request.
    func testBackgroundModelJobsRunOnTheirOwnLaneAndWaitForTheUser() throws {
        let text = try appState()
        for step in ["scheduleJobsBatchStep", "scheduleEpisodeSegmentStep", "scheduleReplyPriorityStep", "scheduleTopicsStep",
                     "scheduleTopicVaultStep", "scheduleTopicStepsStep", "scheduleTopicNamingStep"] {
            let b = try tightBody(text, "private func \(step)(")
            XCTAssertFalse(b.contains("CoreMemoryStore.slowLaneQueue"), "\(step) must not use the user's question lane")
            XCTAssertTrue(b.contains("backgroundModelLane") || b.contains("modelQueue"), step)
        }
        for step in ["scheduleJobsBatchStep", "scheduleEpisodeSegmentStep", "scheduleTopicStepsStep", "scheduleTopicNamingStep"] {
            XCTAssertTrue(try tightBody(text, "private func \(step)(").contains("UserModelGate.shared.isBusy()"), "\(step) waits for the user")
        }
        XCTAssertTrue(text.contains("backgroundModelStore = coreStore.flatMap"))
        XCTAssertTrue(text.contains("CoreMemoryStore.backgroundModelQueue.sync { _ = lanes.model?.setBusyTimeout() }"))
        let compaction = try String(contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel/Support/StorageFootprint.swift"), encoding: .utf8)
        XCTAssertTrue(compaction.contains("CoreMemoryStore.backgroundModelQueue.sync"), "compaction holds the new lane too")
    }

    /// The excluded-client check runs first on the question's own lane (never `accessQueue`: with an excluded
    /// client it can rescan); the picked model is named only once it says the question is not excluded.
    func testAskChecksExclusionOnItsOwnLaneBeforeNamingAModel() throws {
        let ask = try String(contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel/UI/TimelineView+Ask.swift"), encoding: .utf8)
        XCTAssertFalse(try tightBody(ask, "private func askNow(").contains("askNamesExcludedClient"), "not on accessQueue")
        let blocking = try tightBody(ask, "func askBlocking(")
        let check = try XCTUnwrap(blocking.range(of: "askNamesExcludedClient"))
        let context = try XCTUnwrap(blocking.range(of: "fetchContextItems"))
        XCTAssertTrue(check.lowerBound < context.lowerBound, "the check is the first thing on the lane")
        XCTAssertTrue(blocking.contains("askModelConfirmed = true"))
        XCTAssertTrue(try tightBody(ask, "func askStreaming(").contains("askModelConfirmed = true"))
    }

    /// The user gate is held for every question, first of a conversation included, through one helper.
    func testAskHoldsTheUserGateThroughSetAskingOnly() throws {
        let ask = try String(contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel/UI/TimelineView+Ask.swift"), encoding: .utf8)
        let assignments = ask.components(separatedBy: "\n").filter { $0.contains("isAsking = ") && !$0.trimmingCharacters(in: .whitespaces).hasPrefix("//") }
        XCTAssertEqual(assignments.count, 1, "only setAsking assigns isAsking: \(assignments)")
        let helper = try tightBody(ask, "func setAsking(")
        XCTAssertTrue(helper.contains("UserModelGate.shared.enter()"))
        XCTAssertTrue(ask.contains(".onDisappear { releaseAskGate() }"))
    }
}
