import XCTest
@testable import Whimbrel

@MainActor
final class LiveQuestionWatcherTests: XCTestCase {
    private final class FakeDelivery: AlertDelivering {
        var events: [String] = []
        var detected: [CoreLiveAlert] = []
        var drafted: [CoreLiveAlert] = []
        var capped: [Int] = []
        func alertDetected(_ alert: CoreLiveAlert) { detected.append(alert); events.append("detected") }
        func alertDrafted(_ alert: CoreLiveAlert, answer: CoreChatAnswer?) { drafted.append(alert); events.append("drafted") }
        func alertsCapped(meetingId: Int64, extra: Int) { capped.append(extra); events.append("capped") }
    }

    private final class Gate { var open = true }

    private var detectCalls: [String] = []
    private var inserted: [CoreNewLiveAlert] = []
    private var delivery = FakeDelivery()

    override func setUp() {
        super.setUp()
        detectCalls = []
        inserted = []
        delivery = FakeDelivery()
    }

    private func row(_ id: Int64, _ channel: String, _ startCs: Int64, _ text: String) -> CoreLiveSegment {
        CoreLiveSegment(id: id, meetingId: 1, channel: channel, chunkIndex: 0, startCs: startCs, endCs: startCs + 100, speakerLabel: "", text: text)
    }

    /// Every candidate the fake model sees is a question; the restatement is the line itself.
    private func makeWatcher(gate: Gate = Gate(), title: String = "Standup", partOffsetCs: Int64 = 0) -> LiveQuestionWatcher {
        LiveQuestionWatcher(
            meetingId: 1, meetingTitle: title, names: ["Mike Pointer", "Mike"],
            detect: { [unowned self] lines, index in
                let text = lines[index].text
                await MainActor.run { self.detectCalls.append(text) }
                return CoreLiveQuestionVerdict(isForUser: true, confidence: "high", quote: text, asker: nil, restatement: text)
            },
            draft: { _, _ in
                while !gate.open { await Task.yield() }
                return CoreChatAnswer(text: "Draft", citations: [])
            },
            deliver: delivery,
            now: { Date(timeIntervalSince1970: 1_000_000) },
            insert: { [unowned self] alert in
                await MainActor.run { self.inserted.append(alert) }
                return Int64(self.inserted.count)
            },
            saveDraft: { _, _ in },
            seed: { [unowned self] in
                // The fake store: every alert inserted so far, by any watcher, and this part's offset.
                await MainActor.run {
                    (self.inserted.enumerated().map { i, a in
                        CoreLiveAlert(
                            id: Int64(i + 1), meetingId: a.meetingId, originDeviceId: nil, sourceId: Int64(i + 1), sourceMeetingId: a.meetingId,
                            meetingTitle: a.meetingTitle, quote: a.quote, restatement: a.restatement, asker: a.asker, atCs: a.atCs,
                            createdAtUnixMs: a.createdAtUnixMs, draftJson: nil, emailStatus: nil, notifiedAtUnixMs: nil
                        )
                    }, partOffsetCs)
                }
            }
        )
    }

    // MARK: - Final review fixes

    func testContinuedMeetingJudgesPartTwoQuestionsInMeetingTime() async {
        // Part 1 alerted at minute 38; part 2 starts at minute 40 and its live rows restart near 0.
        inserted = [CoreNewLiveAlert(meetingId: 1, meetingTitle: "Standup", quote: "Mike, budget?", restatement: "Budget",
                                     asker: nil, atCs: 228_000, createdAtUnixMs: 1)]
        let watcher = makeWatcher(partOffsetCs: 240_000)
        watcher.ingest([row(10, "system", 6_000, "Mike, can you send the deck?")])
        watcher.flush()
        await watcher.settle()
        XCTAssertEqual(detectCalls, ["Mike, can you send the deck?"], "a part-2 question is not behind part 1's last alert")
        XCTAssertEqual(inserted.last?.atCs, 246_000, "stored meeting-absolute: minute 41")
        XCTAssertEqual(delivery.detected.last?.atCs, 246_000)
    }

    func testPartOffsetIsLatestPartFromFirst() {
        func part(_ index: Int64, _ startMs: Int64) -> CoreMeetingPart {
            CoreMeetingPart(partIndex: index, startUnixMs: startMs, endUnixMs: nil, audioPath: nil, transcriptPath: nil, transcribed: false)
        }
        XCTAssertEqual(LiveQuestionWatcher.partOffsetCs([]), 0)
        XCTAssertEqual(LiveQuestionWatcher.partOffsetCs([part(1, 1_000_000)]), 0)
        XCTAssertEqual(LiveQuestionWatcher.partOffsetCs([part(2, 3_400_000), part(1, 1_000_000)]), 240_000)
    }

    func testMicLinesWaitForSystemAudioOrAMinute() async {
        let watcher = makeWatcher()
        watcher.ingest([row(1, "mic", 0, "Mike here, can you hear me?"), row(2, "mic", 3_500, "Okay.")])
        await watcher.settle()
        XCTAssertEqual(detectCalls, [], "the user's own mic before system audio arrives")
        watcher.ingest([row(3, "mic", 7_000, "Still just me in this room.")])
        watcher.flush()
        await watcher.settle()
        XCTAssertEqual(detectCalls, ["Mike here, can you hear me?"], "a minute with no system audio: room audio")
    }

    func testRoomAudioTitleCountsTheMicAtOnce() async {
        let watcher = makeWatcher(title: "Room audio only · Planning")
        watcher.ingest([row(1, "mic", 0, "Mike, can you send the deck?")])
        watcher.flush()
        await watcher.settle()
        XCTAssertEqual(detectCalls, ["Mike, can you send the deck?"])
    }

    func testRebuiltWatcherDoesNotRealertOrResetCeiling() async {
        let rows = [row(1, "system", 0, "Mike, budget?"), row(2, "system", 1_000, "Mike, hiring?"), row(3, "system", 9_000, "Next item.")]
        let first = makeWatcher()
        first.ingest(rows)
        first.flush()
        await first.settle()
        XCTAssertEqual(detectCalls.count, 2)
        XCTAssertEqual(delivery.detected.count, 2)

        let rebuilt = makeWatcher()
        rebuilt.ingest(rows)
        rebuilt.flush()
        await rebuilt.settle()
        XCTAssertEqual(detectCalls.count, 2, "candidates at or before the last stored alert are not judged again")
        XCTAssertEqual(delivery.detected.count, 2)
        XCTAssertEqual(rebuilt.alertCount, 2)
    }

    func testTranscriptionWantedNeedsANameForAlerts() {
        XCTAssertTrue(AppState.liveTranscriptionWanted(transcript: true, alerts: false, names: []))
        XCTAssertTrue(AppState.liveTranscriptionWanted(transcript: false, alerts: true, names: ["Mike"]))
        XCTAssertFalse(AppState.liveTranscriptionWanted(transcript: false, alerts: true, names: []))
        XCTAssertFalse(AppState.liveTranscriptionWanted(transcript: false, alerts: false, names: ["Mike"]))
    }

    func testWatcherSkipsSegmentsAlreadySeen() async {
        let watcher = makeWatcher()
        let rows = [row(1, "system", 0, "Mike, budget?"), row(2, "system", 1_000, "Okay."), row(3, "system", 4_000, "Next item.")]
        watcher.ingest(rows)
        watcher.ingest(rows)
        await watcher.settle()
        XCTAssertEqual(detectCalls, ["Mike, budget?"])
        watcher.flush()
        await watcher.settle()
        XCTAssertEqual(detectCalls, ["Mike, budget?"])
    }

    func testMicLinesNeverTrigger() async {
        let watcher = makeWatcher()
        watcher.ingest([row(1, "system", 0, "Hello all."), row(2, "mic", 1_000, "Mike here, can you hear me?"), row(3, "system", 5_000, "Yes.")])
        watcher.flush()
        await watcher.settle()
        XCTAssertEqual(detectCalls, [])
        XCTAssertTrue(delivery.detected.isEmpty)
    }

    func testCandidateWaitsForThirtySecondsOfFollowingLines() async {
        let watcher = makeWatcher()
        watcher.ingest([row(1, "system", 10_000, "Mike, budget?"), row(2, "system", 12_999, "Hmm.")])
        await watcher.settle()
        XCTAssertEqual(detectCalls, [])
        watcher.ingest([row(3, "system", 13_000, "Anyway.")])
        await watcher.settle()
        XCTAssertEqual(detectCalls, ["Mike, budget?"])
    }

    func testFlushJudgesCandidatesStillWaiting() async {
        let watcher = makeWatcher()
        watcher.ingest([row(1, "system", 0, "Mike, budget?")])
        await watcher.settle()
        XCTAssertEqual(detectCalls, [])
        watcher.flush()
        await watcher.settle()
        XCTAssertEqual(detectCalls, ["Mike, budget?"])
        XCTAssertEqual(delivery.detected.map(\.quote), ["Mike, budget?"])
        XCTAssertEqual(inserted.first?.meetingTitle, "Standup")
    }

    func testDuplicateWithinFiveMinutesAlertsOnce() async {
        let watcher = makeWatcher()
        watcher.ingest([row(1, "system", 0, "Mike, can you send the budget?"), row(2, "system", 2_000, "Mike, can you send the budget?")])
        watcher.flush()
        await watcher.settle()
        XCTAssertEqual(detectCalls.count, 2)
        XCTAssertEqual(delivery.detected.count, 1)
        XCTAssertEqual(inserted.count, 1)
    }

    func testCeilingSixThenOneCappedCall() async {
        let watcher = makeWatcher()
        let topics = ["budget", "hiring", "roadmap", "pricing", "launch", "security", "travel", "office"]
        watcher.ingest(topics.enumerated().map { i, t in row(Int64(i + 1), "system", Int64(i) * 1_000, "Mike, \(t)?") })
        watcher.flush()
        await watcher.settle()
        XCTAssertEqual(delivery.detected.count, 6)
        XCTAssertEqual(delivery.capped, [2])
        XCTAssertEqual(inserted.count, 8, "capped alerts are still stored for the meeting page")
    }

    func testNoNamesNoWatcher() {
        XCTAssertEqual(ProfileSettings.names(profileName: "", otherNames: ""), [])
        XCTAssertEqual(ProfileSettings.names(profileName: " Mike Pointer ", otherNames: "MP, Mikey,, mike"), ["Mike Pointer", "Mike", "MP", "Mikey"])
        let defaults = UserDefaults(suiteName: "LiveQuestionWatcherTests-\(UUID().uuidString)")!
        let settings = LiveQuestionSettings(defaults: defaults)
        XCTAssertTrue(settings.enabled)
        XCTAssertTrue(settings.emailEnabled)
        XCTAssertFalse(LiveQuestionWatcher.shouldRun(settings: settings, names: []))
        XCTAssertTrue(LiveQuestionWatcher.shouldRun(settings: settings, names: ["Mike"]))
        settings.enabled = false
        XCTAssertFalse(LiveQuestionWatcher.shouldRun(settings: settings, names: ["Mike"]))
        XCTAssertFalse(LiveQuestionSettings(defaults: defaults).enabled)
    }

    func testDraftNeverDelaysDetection() async {
        let gate = Gate()
        gate.open = false
        let watcher = makeWatcher(gate: gate)
        watcher.ingest([row(1, "system", 0, "Mike, budget?"), row(2, "system", 1_000, "Mike, hiring?")])
        watcher.flush()
        await watcher.settleDetection()
        XCTAssertEqual(delivery.detected.count, 2)
        XCTAssertTrue(delivery.drafted.isEmpty)
        gate.open = true
        await watcher.settle()
        XCTAssertEqual(delivery.drafted.count, 2)
        XCTAssertEqual(delivery.events, ["detected", "detected", "drafted", "drafted"])
    }

    func testDetectEnvelopeWithNullDataIsNoQuestion() {
        guard case .success(let none) = CoreLiveQuestion.detectResult(#"{"ok":true,"data":null}"#) else { return XCTFail() }
        XCTAssertNil(none)
        let json = #"{"ok":true,"data":{"is_for_user":true,"confidence":"high","quote":"Mike?","asker":null,"restatement":"Reply"}}"#
        guard case .success(let verdict) = CoreLiveQuestion.detectResult(json) else { return XCTFail() }
        XCTAssertEqual(verdict?.quote, "Mike?")
        XCTAssertEqual(CoreLiveQuestion.detectResult(#"{"ok":false,"error":{"kind":"provider_failure"}}"#), .failure(.providerFailure))
    }
}
