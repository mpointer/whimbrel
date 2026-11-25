import WhimbrelUI
import XCTest
@testable import Whimbrel

/// The live meeting adapter and the recording notice default, flag on and off. Nothing here records or reads real audio.
final class LiveMeetingRedesignTests: XCTestCase {
    private func defaults() -> UserDefaults {
        let d = UserDefaults(suiteName: "LiveMeetingRedesignTests")!
        d.removePersistentDomain(forName: "LiveMeetingRedesignTests")
        return d
    }

    // MARK: Notice default

    func testWithTheFlagOnAnAbsentSettingMeansOn() {
        XCTAssertTrue(RecordingNoticeSettings(defaults: defaults(), redesign: true).enabled)
    }

    func testTheAppsOwnValueIsOnWithNothingStoredAndAStoredOffStays() {
        let d = defaults()
        XCTAssertTrue(RecordingNoticeSettings(defaults: d, redesign: RedesignFlag.isOn()).enabled)
        RecordingNoticeSettings(defaults: d, redesign: RedesignFlag.isOn()).enabled = false
        XCTAssertFalse(RecordingNoticeSettings(defaults: d, redesign: RedesignFlag.isOn()).enabled)
    }

    func testWithTheFlagOffAnAbsentSettingStaysOff() {
        XCTAssertFalse(RecordingNoticeSettings(defaults: defaults(), redesign: false).enabled)
    }

    func testAStoredOffWinsWithTheFlagOn() {
        let d = defaults()
        let settings = RecordingNoticeSettings(defaults: d, redesign: true)
        settings.enabled = false
        XCTAssertFalse(RecordingNoticeSettings(defaults: d, redesign: true).enabled)
        settings.enabled = true
        XCTAssertTrue(RecordingNoticeSettings(defaults: d, redesign: false).enabled)
    }

    // MARK: State, time and text

    private let start = Date(timeIntervalSince1970: 1_800_000_000)

    func testTheStateFollowsThePauseModel() {
        var pause = RecordingPauseModel(startedAt: start)
        func state(resuming: Bool = false, problem: String? = nil) -> WLiveState {
            LiveMeetingAdapter.state(isProcessing: false, stage: nil, isStarting: false, pause: pause, resuming: resuming, problem: problem)
        }
        XCTAssertEqual(state(), .recording)
        pause.apply(.pause, now: start.addingTimeInterval(60))
        XCTAssertEqual(state(), .paused)
        XCTAssertEqual(state(resuming: true), .resuming)
        pause.apply(.resumeFailed, now: start.addingTimeInterval(90))
        XCTAssertEqual(state(problem: "Microphone not found"), .problem("Microphone not found"))
        XCTAssertEqual(LiveMeetingAdapter.state(isProcessing: true, stage: "Transcribing", isStarting: false, pause: nil, resuming: false, problem: nil), .processing(stage: "Transcribing"))
        XCTAssertEqual(LiveMeetingAdapter.state(isProcessing: false, stage: nil, isStarting: true, pause: nil, resuming: false, problem: nil), .starting)
    }

    func testRecordedTimeLeavesOutThePause() {
        var pause = RecordingPauseModel(startedAt: start)
        pause.apply(.pause, now: start.addingTimeInterval(100))
        let now = start.addingTimeInterval(400)
        XCTAssertEqual(LiveMeetingAdapter.recordedSeconds(now: now, startedAt: start, pause: pause), 100)
        pause.apply(.resume, now: now)
        XCTAssertEqual(LiveMeetingAdapter.recordedSeconds(now: now.addingTimeInterval(50), startedAt: start, pause: pause), 150)
        XCTAssertEqual(LiveMeetingAdapter.recordedSeconds(now: now, startedAt: start, pause: nil), 400)
    }

    func testTheLabelIsTheLastNotesLine() {
        XCTAssertEqual(LiveMeetingAdapter.lastLine(of: "one\ntwo\n  three  \n\n"), "three")
        XCTAssertEqual(LiveMeetingAdapter.lastLine(of: ""), "")
    }

    func testTheTimeLineHasNextNowAndStarted() {
        let stops = LiveMeetingAdapter.stops(next: (start.addingTimeInterval(7200), "Design sync"), now: start, startedAt: start.addingTimeInterval(-900), started: "Started", nowWord: "Now")
        XCTAssertEqual(stops.map(\.kind), [.next, .now, .started])
        XCTAssertEqual(LiveMeetingAdapter.stops(next: nil, now: start, startedAt: nil, started: "Started", nowWord: "Now").map(\.kind), [.now])
    }

    func testMarksShowTheClockTimeOfDay() {
        let marker = CoreMeetingMarker(id: 7, meetingId: 1, atCs: 6000, label: "Priya: check churn", createdUnixMs: 1_800_000_000_000)
        let marks = LiveMeetingAdapter.marks([marker], locale: Locale(identifier: "en_US_POSIX"))
        XCTAssertEqual(marks.first?.id, "7")
        XCTAssertEqual(marks.first?.label, "Priya: check churn")
        XCTAssertFalse(marks.first?.timeOfDay.isEmpty ?? true)
    }

    /// Mike, 2026-11-25: on 0.9.82 every line one speaker said within a minute showed the text of that minute's first line,
    /// because rows were identified by the minute and the speaker. Each row is now identified by its stored line.
    func testLinesInTheSameMinuteKeepTheirOwnIdAndText() {
        func seg(_ id: Int64, _ startCs: Int64, _ label: String, _ text: String) -> CoreLiveSegment {
            CoreLiveSegment(id: id, meetingId: 1, channel: label == "me" ? "mic" : "system", chunkIndex: 0, startCs: startCs, endCs: startCs + 300, speakerLabel: label, text: text)
        }
        let segments = [
            seg(1, 0, "Others", "That's just kind of the, the,"),
            seg(2, 400, TranscriptionService.Speaker.me.rawValue, "The naming convention that some in the industry are using."),
            seg(3, 800, "Others", "I'm actually taking some PTO next week."),
            seg(4, 1200, TranscriptionService.Speaker.me.rawValue, "As soon as I get back I'll try it."),
        ]
        let entries = LiveMeetingAdapter.entries(segments: segments, spans: [], recordingStartedAt: start, lastLine: .live, you: "You", locale: Locale(identifier: "en_US_POSIX"))
        XCTAssertEqual(Set(entries.map(\.time)).count, 1, "the four lines fall in one minute")
        XCTAssertEqual(Set(entries.map(\.id)).count, 4)
        XCTAssertEqual(entries.map(\.text), segments.map(\.text))
    }

    // MARK: Jump and citation targets

    private func segs(_ n: Int) -> [CoreLiveSegment] {
        (1...n).map { i in
            CoreLiveSegment(id: Int64(i), meetingId: 1, channel: "system", chunkIndex: 0, startCs: Int64(i * 100), endCs: Int64(i * 100 + 90), speakerLabel: "Others", text: "line \(i)")
        }
    }

    private func ids(_ highlighted: String?, count: Int = 60) -> [String] {
        LiveMeetingAdapter.entries(
            segments: segs(count), spans: [], recordingStartedAt: start, lastLine: .normal, you: "You", highlightedId: highlighted,
            locale: Locale(identifier: "en_US_POSIX")
        ).map(\.id)
    }

    func testEntryIdsAreLiveAndTheSegmentId() {
        XCTAssertEqual(LiveMeetingAdapter.entryId(segmentId: 42), "live-42")
        XCTAssertEqual(ids(nil).first, "live-41")
        XCTAssertEqual(ids(nil).last, "live-60")
    }

    func testWithNoHighlightOnlyTheNewestTwentyShow() {
        XCTAssertEqual(ids(nil).count, 20)
    }

    func testAHighlightInsideTheWindowChangesNothing() {
        XCTAssertEqual(ids("live-55"), ids(nil))
    }

    func testAHighlightOlderThanTheWindowWidensItToIncludeThatLine() {
        let shown = ids("live-10")
        XCTAssertTrue(shown.contains("live-10"))
        XCTAssertEqual(shown.first, "live-8", "two lines of context above the target")
        XCTAssertEqual(shown.last, "live-60", "the newest line stays")
        XCTAssertEqual(shown.count, 53)
    }

    func testAHighlightNearTheStartDoesNotGoBelowTheFirstLine() {
        XCTAssertEqual(ids("live-1").first, "live-1")
    }

    func testAHighlightThatNamesNoLineLeavesTheWindowAlone() {
        XCTAssertEqual(ids("live-999"), ids(nil))
    }

    func testTheHighlightLastsAFewSeconds() {
        XCTAssertEqual(LiveMeetingAdapter.highlightDuration, .seconds(8))
    }
}
